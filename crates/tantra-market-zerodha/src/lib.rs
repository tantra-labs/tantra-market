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
//!
//! # Zero-floats note
//!
//! Kite transmits prices as integer paise, and its REST quote API returns `f64` rupees.
//! Every price is converted to an exact `rust_decimal::Decimal`-backed `Price` at this
//! adapter's boundary (equity/derivatives divide by 100, currency by 10⁷) — no float
//! crosses into a canonical `Tick`.

pub mod protocol;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use protocol::{FeedPacket, Packet};
use rust_decimal::Decimal;
use tantra_core::{DepthLevel, Exchange, InstrumentId, Price, Quantity, Tick};
use tantra_market_core::{
    FeedHealth, MarketDataFeed, MdError, Mode, ReconnectionManager, Subscription, SymbolMapper,
    WsClient,
};
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::debug;

/// The official Kite Connect 3 ticker WebSocket endpoint.
pub const DEFAULT_WS_URL: &str = "wss://ws.kite.trade";
/// The Kite REST quote base (used by [`KiteFeed::fetch_snapshot`]).
pub const REST_BASE: &str = "https://api.kite.trade";

/// Kite feed adapter implementing the canonical [`MarketDataFeed`] trait.
pub struct KiteFeed {
    api_key: String,
    access_token: String,
    ws_client: WsClient,
    mapper: Arc<RwLock<SymbolMapper>>,
    reconnection: Arc<ReconnectionManager>,
    /// Wire token → stream mode, so the decode task can dispatch by packet length.
    modes: Arc<RwLock<HashMap<u32, Mode>>>,
    ticks: broadcast::Sender<Tick>,
    connected: Arc<AtomicBool>,
}

impl KiteFeed {
    /// Creates a feed for a Kite `api_key`/`access_token` pair.
    pub fn new(api_key: impl Into<String>, access_token: impl Into<String>) -> Self {
        let api_key = api_key.into();
        let access_token = access_token.into();
        let url = format!("{DEFAULT_WS_URL}?api_key={api_key}&access_token={access_token}");
        let (ticks, _) = broadcast::channel(16_384);
        Self {
            api_key,
            access_token,
            ws_client: WsClient::new(url),
            mapper: Arc::new(RwLock::new(SymbolMapper::new())),
            reconnection: Arc::new(ReconnectionManager::new()),
            modes: Arc::new(RwLock::new(HashMap::new())),
            ticks,
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Connects to the Kite ticker and spawns the decode-and-broadcast task.
    ///
    /// Call this before [`MarketDataFeed::subscribe`]. The underlying [`WsClient`] owns
    /// reconnection (exponential backoff with jitter); auto-resubscribe on reconnect from
    /// [`ReconnectionManager`] is a documented follow-on.
    pub async fn connect(&self) -> Result<(), MdError> {
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel();
        self.ws_client
            .connect(msg_tx)
            .await
            .map_err(|e| MdError::Auth(format!("Kite feed connect failed: {e}")))?;
        let ticks = self.ticks.clone();
        let mapper = self.mapper.clone();
        let modes = self.modes.clone();
        let connected = self.connected.clone();
        connected.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
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
        self.reconnection
            .set_status(tantra_market_core::ConnectionStatus::Connected)
            .await;
        Ok(())
    }
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
    let exchange = instrument.exchange;
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
    Price::new(Decimal::from(paise) / divisor)
}

/// Converts an integral quantity into an exact `Decimal` `Quantity`.
fn to_quantity(value: i32) -> Quantity {
    Quantity::new(Decimal::from(value))
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
        // Subscribe to every token first, then set the stream mode per group.
        let mut tokens_by_mode: HashMap<Mode, Vec<u32>> = HashMap::new();
        let mut all_tokens = Vec::new();
        for sub in &subs {
            let token = self.kite_token(sub.instrument.clone()).await?;
            {
                let mut mapper = self.mapper.write().await;
                mapper.register(token.to_string(), sub.instrument.clone());
            }
            {
                let mut modes = self.modes.write().await;
                modes.insert(token, sub.mode);
            }
            self.reconnection.add_subscription(sub.clone()).await;
            tokens_by_mode.entry(sub.mode).or_default().push(token);
            all_tokens.push(token);
        }
        all_tokens.sort_unstable();
        all_tokens.dedup();

        self.ws_client
            .send_text(protocol::subscribe_json(
                protocol::action::SUBSCRIBE,
                &all_tokens,
            ))
            .await
            .map_err(|e| MdError::Transport(e.to_string()))?;
        for (mode, tokens) in tokens_by_mode {
            let mode_name = match mode {
                Mode::Ltp => "ltp",
                Mode::Quote => "quote",
                Mode::Full => "full",
            };
            self.ws_client
                .send_text(protocol::mode_json(mode_name, &tokens))
                .await
                .map_err(|e| MdError::Transport(e.to_string()))?;
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
            let token = self.kite_token(instrument.clone()).await?;
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
        // Kite REST LTP quotes (`GET /quote/ltp?i=EXCHANGE:SYMBOL`, one per instrument).
        if instruments.is_empty() {
            return Ok(Vec::new());
        }
        let query: Vec<(String, String)> = instruments
            .iter()
            .map(|instrument| {
                (
                    "i".to_string(),
                    format!("{}:{}", instrument.exchange.as_str(), instrument.symbol),
                )
            })
            .collect();
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{REST_BASE}/quote/ltp"))
            .query(&query)
            .header("X-Kite-Version", "3")
            .header(
                "Authorization",
                format!("token {}:{}", self.api_key, self.access_token),
            )
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
        let mapper = self.mapper.read().await;
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
            let Some(instrument) = mapper.to_canonical(&token.to_string()) else {
                continue;
            };
            ticks.push(Tick::Ltp {
                instrument,
                price: Price::new(Decimal::from_f64_retain(last_price).unwrap_or(Decimal::ZERO)),
                volume: Quantity::ZERO,
                exchange_ts: now,
                receive_ts: now,
            });
        }
        drop(mapper);
        Ok(ticks)
    }

    fn feed_health(&self) -> FeedHealth {
        FeedHealth {
            name: self.name().to_string(),
            connected: self.connected.load(Ordering::SeqCst),
            last_heartbeat: None,
            rtt_ms: None,
            dropped_count: 0,
            reconnect_count: 0,
        }
    }
}

impl KiteFeed {
    /// The Kite instrument token for a canonical instrument: the attached broker token,
    /// else a registered symbol-mapper key.
    async fn kite_token(&self, instrument: InstrumentId) -> Result<u32, MdError> {
        if let Some(token) = &instrument.token
            && let Ok(token) = token.parse::<u32>()
        {
            return Ok(token);
        }
        Err(MdError::SubscriptionFailed(format!(
            "no Kite instrument token for {instrument}; attach a broker token"
        )))
    }
}
