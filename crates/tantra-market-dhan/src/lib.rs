//! DhanHQ Live Market Feed adapter (`tantra-market/docs/gaps/0003-broker-websocket-feed-stubs.md`).
//!
//! A real [`MarketDataFeed`] over Dhan's v2 WebSocket tickfeed
//! (`wss://api-feed.dhan.co?version=2&token=...&clientId=...&authType=2`), implemented from
//! the official DhanHQ v2 documentation:
//!
//! - Requests are JSON; every response is a little-endian binary packet ([`protocol`]).
//! - Subscribe/unsubscribe are chunked at 100 instruments per JSON message and keyed by
//!   [`Mode`]: `Ltp` → ticker (15), `Quote` → quote (17), `Full` → full (21).
//! - Decoded packets are normalized to canonical [`Tick`]s: ticker/quote → `Tick::Ltp`,
//!   full → `Tick::Depth` (5 levels). OI and prev-close packets are *not* ticks; wiring
//!   them into the OI snapshot store (`tantra-market/docs/gaps/0002`) is a documented
//!   follow-on.
//! - Reconnects refresh credentials dynamically via [`TokenProvider`], replay active
//!   subscriptions from [`ReconnectionManager`], and invoke REST [`MarketDataFeed::fetch_snapshot`]
//!   reconciliation to fill missed ticks.
//!
//! # Zero-floats note
//!
//! Dhan transmits prices as `f32` floats (a documented exception to the zero-floats
//! policy, confined to this adapter's boundary): every price/quantity is converted to an
//! exact `rust_decimal::Decimal`-backed `Price`/`Quantity` before it is placed in a
//! canonical `Tick`. No float crosses into domain code.

pub mod protocol;

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use protocol::{
    DhanInstrument, ExchangeSegment, FeedPacket, ResponseHeader, request, subscribe_json,
};
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

/// The official DhanHQ v2 market-feed WebSocket endpoint.
pub const DEFAULT_WS_URL: &str = "wss://api-feed.dhan.co";
/// The DhanHQ v2 market-quote REST base (used by [`DhanFeed::fetch_snapshot`]).
pub const MARKET_QUOTE_URL: &str = "https://api.dhan.co/v2/marketfeed/ltp";

/// Dynamic DhanHQ v2 [`WsHandshake`] building URL query auth on every (re)connect.
struct DhanHandshake {
    client_id: String,
    ws_base_url: String,
    token_provider: Arc<dyn TokenProvider>,
}

#[async_trait]
impl WsHandshake for DhanHandshake {
    async fn prepare(&self) -> WsResult<WsConnectTarget> {
        let token = self
            .token_provider
            .access_token()
            .await
            .map_err(|e| WsError::Connection(e.to_string()))?;
        let url = format!(
            "{}?version=2&token={token}&clientId={}&authType=2",
            self.ws_base_url, self.client_id
        );
        Ok(WsConnectTarget::new(url))
    }
}

/// Dhan feed adapter implementing the canonical [`MarketDataFeed`] trait.
pub struct DhanFeed {
    client_id: String,
    ws_base_url: String,
    rest_quote_url: String,
    token_provider: Arc<dyn TokenProvider>,
    static_token: Option<Arc<StaticTokenProvider>>,
    ws_client: Arc<WsClient>,
    mapper: Arc<RwLock<SymbolMapper>>,
    reconnection: Arc<ReconnectionManager>,
    ticks: broadcast::Sender<Tick>,
    decode_task: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl fmt::Debug for DhanFeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let health = self.ws_client.health_snapshot();
        f.debug_struct("DhanFeed")
            .field("client_id", &self.client_id)
            .field(
                "ws_base_url",
                &tantra_market_core::redact_url(&self.ws_base_url),
            )
            .field(
                "rest_quote_url",
                &tantra_market_core::redact_url(&self.rest_quote_url),
            )
            .field("connected", &health.connected)
            .field("reconnect_count", &health.reconnect_count)
            .finish()
    }
}

impl Drop for DhanFeed {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.decode_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
    }
}

impl DhanFeed {
    /// Creates a feed for a Dhan `client_id`/`access_token`.
    pub fn new(client_id: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self::with_url(client_id, access_token, DEFAULT_WS_URL)
    }

    /// Creates a feed for an explicit WebSocket URL (defaults to [`DEFAULT_WS_URL`]).
    ///
    /// The access token and client id are supplied on every handshake via [`WsHandshake`]
    /// (`?version=2&token=...&clientId=...&authType=2`) and never logged verbatim.
    pub fn with_url(
        client_id: impl Into<String>,
        access_token: impl Into<String>,
        ws_url: impl Into<String>,
    ) -> Self {
        Self::with_ws_config(client_id, access_token, ws_url, WsConfig::default())
    }

    /// Creates a feed with a mutable static token and custom [`WsConfig`].
    pub fn with_ws_config(
        client_id: impl Into<String>,
        access_token: impl Into<String>,
        ws_url: impl Into<String>,
        ws_config: WsConfig,
    ) -> Self {
        let static_provider = Arc::new(StaticTokenProvider::new(access_token));
        Self::build(
            client_id.into(),
            ws_url.into(),
            MARKET_QUOTE_URL.to_string(),
            static_provider.clone(),
            Some(static_provider),
            ws_config,
        )
    }

    /// Creates a feed with explicit WebSocket and REST endpoints, a custom dynamic
    /// [`TokenProvider`], and custom [`WsConfig`].
    pub fn with_config(
        client_id: impl Into<String>,
        ws_url: impl Into<String>,
        rest_quote_url: impl Into<String>,
        token_provider: Arc<dyn TokenProvider>,
        ws_config: WsConfig,
    ) -> Self {
        Self::build(
            client_id.into(),
            ws_url.into(),
            rest_quote_url.into(),
            token_provider,
            None,
            ws_config,
        )
    }

    fn build(
        client_id: String,
        ws_base_url: String,
        rest_quote_url: String,
        token_provider: Arc<dyn TokenProvider>,
        static_token: Option<Arc<StaticTokenProvider>>,
        ws_config: WsConfig,
    ) -> Self {
        let handshake = Arc::new(DhanHandshake {
            client_id: client_id.clone(),
            ws_base_url: ws_base_url.clone(),
            token_provider: token_provider.clone(),
        });
        let ws_client = Arc::new(WsClient::from_handshake(handshake, ws_config));
        let mapper = Arc::new(RwLock::new(SymbolMapper::new()));
        let reconnection = Arc::new(ReconnectionManager::new());
        let (ticks, _) = broadcast::channel(DECODE_QUEUE_CAPACITY);

        // Register automatic subscription replay + REST snapshot reconciliation hook.
        let weak_ws = Arc::downgrade(&ws_client);
        let hook_reconnection = reconnection.clone();
        let hook_mapper = mapper.clone();
        let hook_ticks = ticks.clone();
        let hook_client_id = client_id.clone();
        let hook_rest_url = rest_quote_url.clone();
        let hook_token_provider = token_provider.clone();
        ws_client.set_on_connected(Arc::new(move |is_reconnect| {
            let weak_ws = weak_ws.clone();
            let reconnection = hook_reconnection.clone();
            let mapper = hook_mapper.clone();
            let ticks = hook_ticks.clone();
            let client_id = hook_client_id.clone();
            let rest_url = hook_rest_url.clone();
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
                send_dhan_subscriptions(&ws, &mapper, &subs)
                    .await
                    .map_err(|e| WsError::Transport(e.to_string()))?;

                if is_reconnect {
                    let mut seen = HashSet::new();
                    let instruments: Vec<InstrumentId> = subs
                        .iter()
                        .filter(|s| seen.insert(s.instrument.clone()))
                        .map(|s| s.instrument.clone())
                        .collect();
                    match fetch_dhan_snapshot(
                        &client_id,
                        &rest_url,
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
                            warn!("Dhan reconnect snapshot gap-fill failed: {e}");
                        }
                    }
                }

                reconnection.set_status(ConnectionStatus::Connected).await;
                Ok(())
            })
        }));

        Self {
            client_id,
            ws_base_url,
            rest_quote_url,
            token_provider,
            static_token,
            ws_client,
            mapper,
            reconnection,
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

    /// Connects to the Dhan tickfeed, awaits the initial handshake, and spawns the
    /// bounded decode-and-broadcast task.
    ///
    /// Call this before [`MarketDataFeed::subscribe`]; the feed errors on subscribe while
    /// disconnected. The underlying [`WsClient`] owns reconnection (exponential backoff
    /// with jitter, dead-connection detection, dynamic token refresh) and triggers automatic
    /// subscription replay plus snapshot gap-fill via [`ReconnectionManager`].
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
            .map_err(|e| MdError::Auth(format!("Dhan feed connect failed: {e}")))?;
        self.ws_client
            .wait_initial_connection()
            .await
            .map_err(|e| MdError::Transport(format!("Dhan feed connect failed: {e}")))?;

        let ticks = self.ticks.clone();
        let mapper = self.mapper.clone();
        let reconnection = self.reconnection.clone();
        let handle = tokio::spawn(async move {
            while let Some(message) = msg_rx.recv().await {
                if message.is_binary {
                    match protocol::decode_packet(&message.payload) {
                        Ok(packet) => {
                            if let FeedPacket::Disconnect { code } = packet {
                                warn!("Dhan feed disconnected by server: code {code}");
                                reconnection
                                    .set_status(ConnectionStatus::Disconnected {
                                        reason: format!("server disconnect code {code}"),
                                    })
                                    .await;
                                continue;
                            }
                            if let Some(tick) =
                                packet_to_tick(&packet, &mapper, message.received_at).await
                            {
                                let _ = ticks.send(tick);
                            }
                        }
                        Err(e) => debug!("dropping undecodable Dhan packet: {e}"),
                    }
                }
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

    /// Disconnect from the Dhan tickfeed and stop background reconnect/decode tasks.
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

    /// The Dhan exchange segment for a canonical exchange.
    fn exchange_segment(exchange: Exchange) -> Result<ExchangeSegment, MdError> {
        Ok(match exchange {
            Exchange::Nse => ExchangeSegment::NseEq,
            Exchange::Nfo => ExchangeSegment::NseFno,
            Exchange::Bse => ExchangeSegment::BseEq,
            Exchange::Mcx => ExchangeSegment::McxComm,
            Exchange::Cds => ExchangeSegment::NseCurrency,
        })
    }

    /// The Dhan security id for an instrument: the broker token when attached, else the
    /// registered symbol-mapper key's token, else an error.
    async fn security_id(
        mapper: &RwLock<SymbolMapper>,
        instrument: &InstrumentId,
    ) -> Result<String, MdError> {
        if let Some(token) = instrument.token() {
            return Ok(token.to_string());
        }
        if let Some(key) = mapper.read().await.to_broker_key(instrument) {
            if let Some((_, token)) = key.split_once('|') {
                return Ok(token.to_string());
            }
            return Ok(key.to_string());
        }
        Err(MdError::SubscriptionFailed(format!(
            "no Dhan security id for {instrument}; attach a broker token or register it in the symbol mapper"
        )))
    }

    /// The subscribe request code for a subscription mode.
    fn request_code(mode: Mode) -> i32 {
        match mode {
            Mode::Ltp => request::SUBSCRIBE_TICKER,
            Mode::Quote => request::SUBSCRIBE_QUOTE,
            Mode::Full => request::SUBSCRIBE_FULL,
        }
    }

    /// The unsubscribe request code for a subscription mode.
    fn unsubscribe_request_code(mode: Mode) -> i32 {
        match mode {
            Mode::Ltp => request::UNSUBSCRIBE_TICKER,
            Mode::Quote => request::UNSUBSCRIBE_QUOTE,
            Mode::Full => request::UNSUBSCRIBE_FULL,
        }
    }
}

async fn send_dhan_subscriptions(
    ws_client: &WsClient,
    mapper: &RwLock<SymbolMapper>,
    subs: &[Subscription],
) -> Result<(), MdError> {
    let mut by_mode: Vec<(Mode, Vec<DhanInstrument>)> = Vec::new();
    for sub in subs {
        let segment = DhanFeed::exchange_segment(sub.instrument.exchange())?;
        let security_id = DhanFeed::security_id(mapper, &sub.instrument).await?;
        {
            let mut m = mapper.write().await;
            m.register(
                format!("{}|{security_id}", segment.code()),
                sub.instrument.clone(),
            );
        }
        if let Some((_, list)) = by_mode.iter_mut().find(|(m, _)| *m == sub.mode) {
            list.push(DhanInstrument {
                exchange_segment: segment.name(),
                security_id,
            });
        } else {
            by_mode.push((
                sub.mode,
                vec![DhanInstrument {
                    exchange_segment: segment.name(),
                    security_id,
                }],
            ));
        }
    }

    for (mode, instruments) in by_mode {
        let request_code = DhanFeed::request_code(mode);
        for chunk in instruments.chunks(protocol::MAX_INSTRUMENTS_PER_MESSAGE) {
            let json = subscribe_json(request_code, chunk)
                .map_err(|e| MdError::SubscriptionFailed(e.to_string()))?;
            ws_client
                .send_text(json)
                .await
                .map_err(|e| MdError::Transport(e.to_string()))?;
        }
    }
    Ok(())
}

async fn fetch_dhan_snapshot(
    client_id: &str,
    rest_quote_url: &str,
    token_provider: &dyn TokenProvider,
    mapper: &RwLock<SymbolMapper>,
    instruments: &[InstrumentId],
) -> Result<Vec<Tick>, MdError> {
    let mut body: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for instrument in instruments {
        let segment = DhanFeed::exchange_segment(instrument.exchange())?;
        let security_id = DhanFeed::security_id(mapper, instrument).await?;
        body.entry(segment.name().to_string())
            .or_default()
            .push(security_id);
    }
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let access_token = token_provider.access_token().await?;
    let client = reqwest::Client::new();
    let response = client
        .post(rest_quote_url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("access-token", access_token)
        .header("client-id", client_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| MdError::Transport(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(MdError::Auth(format!(
            "Dhan market-quote snapshot failed with HTTP {status}"
        )));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|e| MdError::Transport(e.to_string()))?;
    let now = Utc::now();
    let mut ticks = Vec::new();
    let mapper_guard = mapper.read().await;
    let Some(data) = payload.get("data").and_then(|d| d.as_object()) else {
        return Err(MdError::Transport(
            "Dhan market-quote response has no data object".into(),
        ));
    };
    for (segment_name, instruments_map) in data {
        let Some(segment_code) =
            ExchangeSegment::from_name(segment_name).map(ExchangeSegment::code)
        else {
            continue;
        };
        let Some(instruments_map) = instruments_map.as_object() else {
            continue;
        };
        for (security_id, quote) in instruments_map {
            let Some(last_price) = quote.get("last_price").and_then(|v| v.as_f64()) else {
                continue;
            };
            let key = format!("{segment_code}|{security_id}");
            let Some(instrument) = mapper_guard.to_canonical(&key) else {
                continue;
            };
            ticks.push(Tick::Ltp {
                instrument,
                price: to_price(last_price),
                volume: Quantity::ZERO,
                exchange_ts: now,
                receive_ts: now,
            });
        }
    }
    drop(mapper_guard);
    Ok(ticks)
}

/// Converts a decoded packet into a canonical tick, resolving the wire instrument via the
/// symbol mapper. Non-tick packets (OI, prev-close, market status, unknown) yield `None`.
pub async fn packet_to_tick(
    packet: &FeedPacket,
    mapper: &RwLock<SymbolMapper>,
    received_at: DateTime<Utc>,
) -> Option<Tick> {
    let header = match packet {
        FeedPacket::Ticker(header, _)
        | FeedPacket::Quote(header, _)
        | FeedPacket::Full(header, _) => header,
        _ => return None,
    };
    let instrument = resolve_instrument(mapper, header).await?;
    let now = received_at;
    match packet {
        FeedPacket::Ticker(_, data) => Some(Tick::Ltp {
            instrument,
            price: to_price(data.last_price),
            volume: Quantity::ZERO,
            exchange_ts: to_epoch(data.last_trade_time),
            receive_ts: now,
        }),
        FeedPacket::Quote(_, data) => Some(Tick::Ltp {
            instrument,
            price: to_price(data.last_price),
            volume: to_quantity(data.volume),
            exchange_ts: to_epoch(data.last_trade_time),
            receive_ts: now,
        }),
        FeedPacket::Full(_, data) => Some(Tick::Depth {
            instrument,
            bids: data
                .depth
                .iter()
                .map(|level| DepthLevel {
                    price: to_price(level.bid_price),
                    quantity: to_quantity(level.bid_qty),
                    orders_count: Some(level.bid_orders.unsigned_abs() as u32),
                })
                .collect(),
            asks: data
                .depth
                .iter()
                .map(|level| DepthLevel {
                    price: to_price(level.ask_price),
                    quantity: to_quantity(level.ask_qty),
                    orders_count: Some(level.ask_orders.unsigned_abs() as u32),
                })
                .collect(),
            last_price: Some(to_price(data.quote.last_price)),
            exchange_ts: to_epoch(data.quote.last_trade_time),
            receive_ts: now,
        }),
        _ => None,
    }
}

async fn resolve_instrument(
    mapper: &RwLock<SymbolMapper>,
    header: &ResponseHeader,
) -> Option<InstrumentId> {
    let key = format!("{}|{}", header.exchange_segment, header.security_id);
    mapper.read().await.to_canonical(&key)
}

/// Rounds an `f32`-sourced price into an exact `Decimal` `Price` at the adapter boundary.
fn to_price(value: f64) -> Price {
    Price::try_new(
        Decimal::from_f64_retain(value)
            .unwrap_or(Decimal::ZERO)
            .max(Decimal::ZERO),
    )
    .unwrap_or(Price::ZERO)
}

/// Converts an integral quantity into an exact `Decimal` `Quantity`.
fn to_quantity(value: i64) -> Quantity {
    Quantity::try_new(Decimal::from(value).max(Decimal::ZERO)).unwrap_or(Quantity::ZERO)
}

/// Converts a unix-epoch seconds value into a UTC instant (epoch when absent).
fn to_epoch(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap_or(DateTime::UNIX_EPOCH)
}

#[async_trait]
impl MarketDataFeed for DhanFeed {
    fn name(&self) -> &str {
        "dhan"
    }

    async fn subscribe(
        &self,
        subs: Vec<Subscription>,
    ) -> Result<BoxStream<'static, Tick>, MdError> {
        if !self.ws_client.is_connected().await {
            return Err(MdError::Transport(
                "Dhan feed is not connected; call DhanFeed::connect() first".into(),
            ));
        }
        send_dhan_subscriptions(&self.ws_client, &self.mapper, &subs).await?;
        for sub in subs {
            self.reconnection.add_subscription(sub).await;
        }

        let stream = BroadcastStream::new(self.ticks.subscribe()).filter_map(|result| result.ok());
        Ok(Box::pin(stream))
    }

    async fn unsubscribe(&self, instruments: &[InstrumentId]) -> Result<(), MdError> {
        for instrument in instruments {
            let resubscribe = self.reconnection.get_resubscribe_list().await;
            let active = resubscribe
                .iter()
                .find(|s| &s.instrument == instrument)
                .cloned();
            let Some(subscription) = active else {
                continue;
            };
            let segment = Self::exchange_segment(subscription.instrument.exchange())?;
            let security_id = Self::security_id(&self.mapper, &subscription.instrument).await?;
            let json = subscribe_json(
                Self::unsubscribe_request_code(subscription.mode),
                &[DhanInstrument {
                    exchange_segment: segment.name(),
                    security_id,
                }],
            )
            .map_err(|e| MdError::SubscriptionFailed(e.to_string()))?;
            self.ws_client
                .send_text(json)
                .await
                .map_err(|e| MdError::Transport(e.to_string()))?;
            self.reconnection.remove_subscription(instrument).await;
        }
        Ok(())
    }

    async fn fetch_snapshot(&self, instruments: &[InstrumentId]) -> Result<Vec<Tick>, MdError> {
        fetch_dhan_snapshot(
            &self.client_id,
            &self.rest_quote_url,
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
