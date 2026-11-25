//! Zerodha Kite Connect 3 WebSocket feed adapter (`tantra-market/docs/gaps/0003-broker-websocket-feed-stubs.md`).
//!
//! A real [`MarketDataFeed`] over Kite's binary ticker (`wss://ws.kite.trade?api_key=...&access_token=...`),
//! implemented from the official Kite Connect 3 WebSocket docs:
//!
//! - Requests are JSON (`{a: "subscribe"|"unsubscribe"|"mode", v: [...]}`); quotes arrive
//!   as little-endian binary messages framing one or more packets ([`protocol`]).
//! - The three stream modes map 1:1 to [`Mode`]: `Ltp` → 8-byte packets, `Quote` → 44-byte
//!   instrument (28-byte index) packets, `Full` → 184-byte instrument (32-byte index)
//!   packets with ten market-depth entries.
//! - Decoded packets are normalized to canonical [`Tick`]s: `ltp`/`quote` → `Tick::Ltp`,
//!   `full` → `Tick::Depth`. OI is carried by full packets but is not a canonical tick;
//!   wiring it into the OI snapshot store (`tantra-market/docs/gaps/0002`) is a documented
//!   follow-on.
//! - Reconnects refresh credentials dynamically via [`TokenProvider`], replay active
//!   subscriptions and stream modes from [`ReconnectionManager`], and invoke REST
//!   [`MarketDataFeed::fetch_snapshot`] reconciliation to fill missed ticks.
//!
//! # Zero-floats note
//!
//! Kite transmits prices as integer paise, and its REST quote API returns `f64` rupees.
//! Every price is converted to an exact `rust_decimal::Decimal`-backed `Price` at this
//! adapter's boundary (equity/derivatives divide by 100, currency by 10⁷) — no float
//! crosses into a canonical `Tick`.

pub mod protocol;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use protocol::{FeedPacket, Packet};
use rust_decimal::Decimal;
use tantra_core::{DepthLevel, Exchange, InstrumentId, Price, Quantity, Tick};
use tantra_market_core::{
    ConnectionStatus, FeedHealth, MarketDataFeed, MdError, Mode, ReconnectionManager,
    StaticTokenProvider, Subscription, SymbolMapper, TokenProvider, WsClient, WsConfig,
    WsConnectTarget, WsError, WsHandshake, WsResult,
};
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, warn};

/// Bounded internal frame queue capacity between [`WsClient`] and decode task.
const DECODE_QUEUE_CAPACITY: usize = 16_384;

/// The official Kite Connect 3 ticker WebSocket endpoint.
pub const DEFAULT_WS_URL: &str = "wss://ws.kite.trade";
/// The Kite REST quote base (used by [`KiteFeed::fetch_snapshot`]).
pub const REST_BASE: &str = "https://api.kite.trade";

/// Dynamic Kite Connect 3 [`WsHandshake`] building query auth on every (re)connect.
struct KiteHandshake {
    api_key: String,
    ws_base_url: String,
    token_provider: Arc<dyn TokenProvider>,
}

#[async_trait]
impl WsHandshake for KiteHandshake {
    async fn prepare(&self) -> WsResult<WsConnectTarget> {
        let access_token = self
            .token_provider
            .access_token()
            .await
            .map_err(|e| WsError::Connection(e.to_string()))?;
        let url = format!(
            "{}?api_key={}&access_token={access_token}",
            self.ws_base_url, self.api_key
        );
        Ok(WsConnectTarget::new(url))
    }
}

/// Kite feed adapter implementing the canonical [`MarketDataFeed`] trait.
pub struct KiteFeed {
    api_key: String,
    ws_base_url: String,
    rest_base_url: String,
    token_provider: Arc<dyn TokenProvider>,
    static_token: Option<Arc<StaticTokenProvider>>,
    ws_client: Arc<WsClient>,
    mapper: Arc<RwLock<SymbolMapper>>,
    reconnection: Arc<ReconnectionManager>,
    /// Wire token → stream mode, so the decode task can dispatch by packet length.
    modes: Arc<RwLock<HashMap<u32, Mode>>>,
    ticks: broadcast::Sender<Tick>,
    decode_task: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl fmt::Debug for KiteFeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let health = self.ws_client.health_snapshot();
        f.debug_struct("KiteFeed")
            .field("api_key", &"[REDACTED]")
            .field(
                "ws_base_url",
                &tantra_market_core::redact_url(&self.ws_base_url),
            )
            .field(
                "rest_base_url",
                &tantra_market_core::redact_url(&self.rest_base_url),
            )
            .field("connected", &health.connected)
            .field("reconnect_count", &health.reconnect_count)
            .finish()
    }
}

impl Drop for KiteFeed {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.decode_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
    }
}

impl KiteFeed {
    /// Creates a feed for a Kite `api_key`/`access_token` pair.
    pub fn new(api_key: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self::with_url(api_key, access_token, DEFAULT_WS_URL)
    }

    /// Creates a feed for an explicit WebSocket URL (defaults to [`DEFAULT_WS_URL`]).
    pub fn with_url(
        api_key: impl Into<String>,
        access_token: impl Into<String>,
        ws_url: impl Into<String>,
    ) -> Self {
        Self::with_ws_config(api_key, access_token, ws_url, WsConfig::default())
    }

    /// Creates a feed with a mutable static token and custom [`WsConfig`].
    pub fn with_ws_config(
        api_key: impl Into<String>,
        access_token: impl Into<String>,
        ws_url: impl Into<String>,
        ws_config: WsConfig,
    ) -> Self {
        let static_provider = Arc::new(StaticTokenProvider::new(access_token));
        Self::build(
            api_key.into(),
            ws_url.into(),
            REST_BASE.to_string(),
            static_provider.clone(),
            Some(static_provider),
            ws_config,
        )
    }

    /// Creates a feed with explicit WebSocket and REST base URLs, a dynamic
    /// [`TokenProvider`], and custom [`WsConfig`].
    pub fn with_config(
        api_key: impl Into<String>,
        ws_url: impl Into<String>,
        rest_base_url: impl Into<String>,
        token_provider: Arc<dyn TokenProvider>,
        ws_config: WsConfig,
    ) -> Self {
        Self::build(
            api_key.into(),
            ws_url.into(),
            rest_base_url.into(),
            token_provider,
            None,
            ws_config,
        )
    }

    fn build(
        api_key: String,
        ws_base_url: String,
        rest_base_url: String,
        token_provider: Arc<dyn TokenProvider>,
        static_token: Option<Arc<StaticTokenProvider>>,
        ws_config: WsConfig,
    ) -> Self {
        let handshake = Arc::new(KiteHandshake {
            api_key: api_key.clone(),
            ws_base_url: ws_base_url.clone(),
            token_provider: token_provider.clone(),
        });
        let ws_client = Arc::new(WsClient::from_handshake(handshake, ws_config));
        let mapper = Arc::new(RwLock::new(SymbolMapper::new()));
        let reconnection = Arc::new(ReconnectionManager::new());
        let modes = Arc::new(RwLock::new(HashMap::new()));
        let (ticks, _) = broadcast::channel(DECODE_QUEUE_CAPACITY);

        let weak_ws = Arc::downgrade(&ws_client);
        let hook_reconnection = reconnection.clone();
        let hook_mapper = mapper.clone();
        let hook_modes = modes.clone();
        let hook_ticks = ticks.clone();
        let hook_api_key = api_key.clone();
        let hook_rest_base = rest_base_url.clone();
        let hook_token_provider = token_provider.clone();
        ws_client.set_on_connected(Arc::new(move |is_reconnect| {
            let weak_ws = weak_ws.clone();
            let reconnection = hook_reconnection.clone();
            let mapper = hook_mapper.clone();
            let modes = hook_modes.clone();
            let ticks = hook_ticks.clone();
            let api_key = hook_api_key.clone();
            let rest_base = hook_rest_base.clone();
            let token_provider = hook_token_provider.clone();
            Box::pin(async move {
                let Some(ws) = weak_ws.upgrade() else {
                    return Ok(());
                };
                let subs = reconnection.get_resubscribe_list().await;
                if subs.is_empty() {
                    reconnection.set_status(ConnectionStatus::Connected).await;
                    return Ok(());
                }
                if is_reconnect {
                    reconnection.set_status(ConnectionStatus::Reconciling).await;
                }
                send_kite_subscriptions(&ws, &mapper, &modes, &subs)
                    .await
                    .map_err(|e| WsError::Transport(e.to_string()))?;

                if is_reconnect {
                    let mut seen = HashSet::new();
                    let instruments: Vec<InstrumentId> = subs
                        .iter()
                        .filter(|s| seen.insert(s.instrument.clone()))
                        .map(|s| s.instrument.clone())
                        .collect();
                    match fetch_kite_snapshot(
                        &api_key,
                        &rest_base,
                        token_provider.as_ref(),
                        &mapper,
                        &instruments,
                    )
                    .await
                    {
                        Ok(snapshot_ticks) => {
                            for tick in snapshot_ticks {
                                let _ = ticks.send(tick);
                            }
                        }
                        Err(e) => {
                            warn!("Kite reconnect snapshot gap-fill failed: {e}");
                        }
                    }
                }

                reconnection.set_status(ConnectionStatus::Connected).await;
                Ok(())
            })
        }));

        Self {
            api_key,
            ws_base_url,
            rest_base_url,
            token_provider,
            static_token,
            ws_client,
            mapper,
            reconnection,
            modes,
            ticks,
            decode_task: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Rotate the static access token used on the next reconnect handshake and snapshot call.
    pub async fn update_access_token(&self, new_token: impl Into<String>) {
        if let Some(static_provider) = &self.static_token {
            static_provider.set_token(new_token).await;
        }
    }

    /// Connects to the Kite ticker, awaits initial handshake, and spawns the bounded
    /// decode-and-broadcast task.
    ///
    /// Call this before [`MarketDataFeed::subscribe`]. The underlying [`WsClient`] owns
    /// reconnection (exponential backoff with jitter, dead-connection timeout, dynamic token
    /// refresh) and triggers automatic subscription replay plus snapshot gap-fill via
    /// [`ReconnectionManager`].
    pub async fn connect(&self) -> Result<(), MdError> {
        if let Ok(mut guard) = self.decode_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }

        let (msg_tx, mut msg_rx) = mpsc::channel(DECODE_QUEUE_CAPACITY);
        self.ws_client
            .connect_bounded(msg_tx)
            .await
            .map_err(|e| MdError::Auth(format!("Kite feed connect failed: {e}")))?;
        self.ws_client
            .wait_initial_connection()
            .await
            .map_err(|e| MdError::Transport(format!("Kite feed connect failed: {e}")))?;

        let ticks = self.ticks.clone();
        let mapper = self.mapper.clone();
        let modes = self.modes.clone();
        let handle = tokio::spawn(async move {
            while let Some(message) = msg_rx.recv().await {
                if message.is_binary {
                    match protocol::decode_message(&message.payload) {
                        Ok(packets) => {
                            for packet in packets {
                                if let Some(tick) =
                                    packet_to_tick(&packet, &modes, &mapper, message.received_at)
                                        .await
                                {
                                    let _ = ticks.send(tick);
                                }
                            }
                        }
                        Err(e) => debug!("dropping undecodable Kite message: {e}"),
                    }
                }
                // Text messages are order postbacks/errors/alerts; they are not ticks and
                // are intentionally ignored by the feed.
            }
        });
        if let Ok(mut guard) = self.decode_task.lock() {
            *guard = Some(handle);
        }

        self.reconnection
            .set_status(ConnectionStatus::Connected)
            .await;
        Ok(())
    }

    /// Disconnect from the Kite ticker and stop background reconnect/decode tasks.
    pub async fn disconnect(&self) {
        if let Ok(mut guard) = self.decode_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
        self.ws_client.disconnect().await;
        self.reconnection
            .set_status(ConnectionStatus::Disconnected {
                reason: "disconnected by caller".into(),
            })
            .await;
    }

    /// The Kite instrument token for a canonical instrument: the attached broker token,
    /// else a registered symbol-mapper key.
    async fn kite_token(
        mapper: &RwLock<SymbolMapper>,
        instrument: &InstrumentId,
    ) -> Result<u32, MdError> {
        if let Some(token) = instrument.token()
            && let Ok(token) = token.parse::<u32>()
        {
            return Ok(token);
        }
        if let Some(key) = mapper.read().await.to_broker_key(instrument)
            && let Ok(token) = key.parse::<u32>()
        {
            return Ok(token);
        }
        Err(MdError::SubscriptionFailed(format!(
            "no Kite instrument token for {instrument}; attach a broker token"
        )))
    }
}

async fn send_kite_subscriptions(
    ws_client: &WsClient,
    mapper: &RwLock<SymbolMapper>,
    modes: &RwLock<HashMap<u32, Mode>>,
    subs: &[Subscription],
) -> Result<(), MdError> {
    let mut tokens_by_mode: HashMap<Mode, Vec<u32>> = HashMap::new();
    let mut all_tokens = Vec::new();
    for sub in subs {
        let token = KiteFeed::kite_token(mapper, &sub.instrument).await?;
        {
            let mut m = mapper.write().await;
            m.register(token.to_string(), sub.instrument.clone());
        }
        {
            let mut mode_map = modes.write().await;
            mode_map.insert(token, sub.mode);
        }
        tokens_by_mode.entry(sub.mode).or_default().push(token);
        all_tokens.push(token);
    }
    all_tokens.sort_unstable();
    all_tokens.dedup();

    ws_client
        .send_text(protocol::subscribe_json(
            protocol::action::SUBSCRIBE,
            &all_tokens,
        ))
        .await
        .map_err(|e| MdError::Transport(e.to_string()))?;
    for (mode, mut tokens) in tokens_by_mode {
        tokens.sort_unstable();
        tokens.dedup();
        let mode_name = match mode {
            Mode::Ltp => "ltp",
            Mode::Quote => "quote",
            Mode::Full => "full",
        };
        ws_client
            .send_text(protocol::mode_json(mode_name, &tokens))
            .await
            .map_err(|e| MdError::Transport(e.to_string()))?;
    }
    Ok(())
}

async fn fetch_kite_snapshot(
    api_key: &str,
    rest_base_url: &str,
    token_provider: &dyn TokenProvider,
    mapper: &RwLock<SymbolMapper>,
    instruments: &[InstrumentId],
) -> Result<Vec<Tick>, MdError> {
    if instruments.is_empty() {
        return Ok(Vec::new());
    }
    let query: Vec<(String, String)> = instruments
        .iter()
        .map(|instrument| {
            (
                "i".to_string(),
                format!("{}:{}", instrument.exchange().as_str(), instrument.symbol()),
            )
        })
        .collect();
    let access_token = token_provider.access_token().await?;
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{rest_base_url}/quote/ltp"))
        .query(&query)
        .header("X-Kite-Version", "3")
        .header("Authorization", format!("token {api_key}:{access_token}"))
        .send()
        .await
        .map_err(|e| MdError::Transport(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(MdError::Auth(format!(
            "Kite LTP snapshot failed with HTTP {status}"
        )));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|e| MdError::Transport(e.to_string()))?;
    let now = Utc::now();
    let mapper_guard = mapper.read().await;
    let mut ticks = Vec::new();
    let Some(data) = payload.get("data").and_then(|d| d.as_object()) else {
        return Err(MdError::Transport(
            "Kite LTP snapshot response has no data object".into(),
        ));
    };
    for quote in data.values() {
        let Some(token) = quote.get("instrument_token").and_then(|v| v.as_u64()) else {
            continue;
        };
        let Some(last_price) = quote.get("last_price").and_then(|v| v.as_f64()) else {
            continue;
        };
        let Some(instrument) = mapper_guard.to_canonical(&token.to_string()) else {
            continue;
        };
        let Some(price) = Decimal::from_f64_retain(last_price).and_then(|d| Price::try_new(d).ok())
        else {
            continue;
        };
        ticks.push(Tick::Ltp {
            instrument,
            price,
            volume: Quantity::ZERO,
            exchange_ts: now,
            receive_ts: now,
        });
    }
    drop(mapper_guard);
    Ok(ticks)
}

/// Converts a decoded packet into a canonical tick, resolving the wire token via the
/// symbol mapper. Packets whose token has no registered mode or mapping are dropped.
pub async fn packet_to_tick(
    packet: &Packet,
    modes: &RwLock<HashMap<u32, Mode>>,
    mapper: &RwLock<SymbolMapper>,
    received_at: DateTime<Utc>,
) -> Option<Tick> {
    let mode = *modes.read().await.get(&packet.token)?;
    let decoded = protocol::decode_packet(mode, packet).ok()?;
    let instrument = {
        let mapper = mapper.read().await;
        mapper.to_canonical(&packet.token.to_string())?
    };
    let exchange = instrument.exchange();
    let now = received_at;
    match decoded {
        FeedPacket::Ltp { last_price, .. } => Some(Tick::Ltp {
            instrument,
            price: to_price(last_price, exchange),
            volume: Quantity::ZERO,
            exchange_ts: now,
            receive_ts: now,
        }),
        FeedPacket::Instrument(quote) if mode == Mode::Quote => Some(Tick::Ltp {
            instrument,
            price: to_price(quote.last_price, exchange),
            volume: to_quantity(quote.volume),
            exchange_ts: to_epoch(quote.last_traded_timestamp),
            receive_ts: now,
        }),
        FeedPacket::Instrument(quote) => {
            let (bids, asks) = match quote.depth {
                Some(depth) => {
                    let bids = depth[..5]
                        .iter()
                        .map(|entry| DepthLevel {
                            price: to_price(entry.price, exchange),
                            quantity: to_quantity(entry.quantity),
                            orders_count: Some(entry.orders.unsigned_abs() as u32),
                        })
                        .collect();
                    let asks = depth[5..]
                        .iter()
                        .map(|entry| DepthLevel {
                            price: to_price(entry.price, exchange),
                            quantity: to_quantity(entry.quantity),
                            orders_count: Some(entry.orders.unsigned_abs() as u32),
                        })
                        .collect();
                    (bids, asks)
                }
                None => (Vec::new(), Vec::new()),
            };
            Some(Tick::Depth {
                instrument,
                bids,
                asks,
                last_price: Some(to_price(quote.last_price, exchange)),
                exchange_ts: to_epoch(quote.exchange_timestamp),
                receive_ts: now,
            })
        }
        FeedPacket::Index(quote) => Some(Tick::Ltp {
            instrument,
            price: to_price(quote.last_price, exchange),
            volume: Quantity::ZERO,
            exchange_ts: to_epoch(quote.exchange_timestamp),
            receive_ts: now,
        }),
    }
}

/// Converts a paise price into an exact `Decimal` `Price` (currency divides by 10⁷).
fn to_price(paise: i32, exchange: Exchange) -> Price {
    let divisor = if exchange == Exchange::Cds {
        Decimal::from(10_000_000)
    } else {
        Decimal::from(100)
    };
    Price::try_new((Decimal::from(paise) / divisor).max(Decimal::ZERO)).unwrap_or(Price::ZERO)
}

/// Converts an integral quantity into an exact `Decimal` `Quantity`.
fn to_quantity(value: i32) -> Quantity {
    Quantity::try_new(Decimal::from(value).max(Decimal::ZERO)).unwrap_or(Quantity::ZERO)
}

/// Converts a unix-epoch seconds value into a UTC instant (epoch when absent).
fn to_epoch(secs: Option<i32>) -> DateTime<Utc> {
    secs.and_then(|s| DateTime::from_timestamp(i64::from(s), 0))
        .unwrap_or(DateTime::UNIX_EPOCH)
}

#[async_trait]
impl MarketDataFeed for KiteFeed {
    fn name(&self) -> &str {
        "zerodha"
    }

    async fn subscribe(
        &self,
        subs: Vec<Subscription>,
    ) -> Result<BoxStream<'static, Tick>, MdError> {
        if !self.ws_client.is_connected().await {
            return Err(MdError::Transport(
                "Kite feed is not connected; call KiteFeed::connect() first".into(),
            ));
        }
        send_kite_subscriptions(&self.ws_client, &self.mapper, &self.modes, &subs).await?;
        for sub in subs {
            self.reconnection.add_subscription(sub).await;
        }

        let stream = BroadcastStream::new(self.ticks.subscribe()).filter_map(|result| result.ok());
        Ok(Box::pin(stream))
    }

    async fn unsubscribe(&self, instruments: &[InstrumentId]) -> Result<(), MdError> {
        let mut tokens = Vec::new();
        for instrument in instruments {
            let resubscribe = self.reconnection.get_resubscribe_list().await;
            if !resubscribe.iter().any(|s| &s.instrument == instrument) {
                continue;
            }
            let token = Self::kite_token(&self.mapper, instrument).await?;
            tokens.push(token);
            {
                let mut modes = self.modes.write().await;
                modes.remove(&token);
            }
            self.reconnection.remove_subscription(instrument).await;
        }
        if tokens.is_empty() {
            return Ok(());
        }
        self.ws_client
            .send_text(protocol::subscribe_json(
                protocol::action::UNSUBSCRIBE,
                &tokens,
            ))
            .await
            .map_err(|e| MdError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn fetch_snapshot(&self, instruments: &[InstrumentId]) -> Result<Vec<Tick>, MdError> {
        fetch_kite_snapshot(
            &self.api_key,
            &self.rest_base_url,
            self.token_provider.as_ref(),
            &self.mapper,
            instruments,
        )
        .await
    }

    fn feed_health(&self) -> FeedHealth {
        let health = self.ws_client.health_snapshot();
        FeedHealth {
            name: self.name().to_string(),
            connected: health.connected,
            last_heartbeat: health.last_heartbeat,
            rtt_ms: health.rtt_ms,
            dropped_count: health.dropped_count,
            reconnect_count: health.reconnect_count,
        }
    }
}
