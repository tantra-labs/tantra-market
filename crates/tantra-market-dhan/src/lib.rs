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
//!
//! # Zero-floats note
//!
//! Dhan transmits prices as `f32` floats (a documented exception to the zero-floats
//! policy, confined to this adapter's boundary): every price/quantity is converted to an
//! exact `rust_decimal::Decimal`-backed `Price`/`Quantity` before it is placed in a
//! canonical `Tick`. No float crosses into domain code.

pub mod protocol;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use protocol::{
    DhanInstrument, ExchangeSegment, FeedPacket, ResponseHeader, request, subscribe_json,
};
use rust_decimal::Decimal;
use tantra_core::{DepthLevel, Exchange, InstrumentId, Price, Quantity, Tick};
use tantra_market_core::{
    FeedHealth, MarketDataFeed, MdError, Mode, ReconnectionManager, Subscription, SymbolMapper,
    WsClient,
};
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, warn};

/// The official DhanHQ v2 market-feed WebSocket endpoint.
pub const DEFAULT_WS_URL: &str = "wss://api-feed.dhan.co";
/// The DhanHQ v2 market-quote REST base (used by [`DhanFeed::fetch_snapshot`]).
pub const MARKET_QUOTE_URL: &str = "https://api.dhan.co/v2/marketfeed/ltp";

/// Dhan feed adapter implementing the canonical [`MarketDataFeed`] trait.
pub struct DhanFeed {
    client_id: String,
    access_token: String,
    ws_client: WsClient,
    mapper: Arc<RwLock<SymbolMapper>>,
    reconnection: Arc<ReconnectionManager>,
    ticks: broadcast::Sender<Tick>,
    connected: Arc<AtomicBool>,
}

impl DhanFeed {
    /// Creates a feed for a Dhan `client_id`/`access_token`.
    pub fn new(client_id: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self::with_url(client_id, access_token, DEFAULT_WS_URL)
    }

    /// Creates a feed for an explicit WebSocket URL (defaults to [`DEFAULT_WS_URL`]).
    ///
    /// The access token and client id are embedded in the connection URL as DhanHQ v2
    /// requires (`?version=2&token=...&clientId=...&authType=2`).
    pub fn with_url(
        client_id: impl Into<String>,
        access_token: impl Into<String>,
        ws_url: impl Into<String>,
    ) -> Self {
        let (client_id, access_token, ws_url) =
            (client_id.into(), access_token.into(), ws_url.into());
        let full_url =
            format!("{ws_url}?version=2&token={access_token}&clientId={client_id}&authType=2");
        let (ticks, _) = broadcast::channel(16_384);
        Self {
            client_id,
            access_token,
            ws_client: WsClient::new(full_url),
            mapper: Arc::new(RwLock::new(SymbolMapper::new())),
            reconnection: Arc::new(ReconnectionManager::new()),
            ticks,
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Connects to the Dhan tickfeed, authenticating via the URL query parameters, and
    /// spawns the decode-and-broadcast task.
    ///
    /// Call this before [`MarketDataFeed::subscribe`]; the feed errors on subscribe while
    /// disconnected. The underlying [`WsClient`] owns reconnection (exponential backoff
    /// with jitter) and replays no subscriptions by itself — [`ReconnectionManager`]
    /// tracks the active set for a follow-on auto-resubscribe on reconnect.
    pub async fn connect(&self) -> Result<(), MdError> {
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel();
        self.ws_client
            .connect(msg_tx)
            .await
            .map_err(|e| MdError::Auth(format!("Dhan feed connect failed: {e}")))?;

        let ticks = self.ticks.clone();
        let mapper = self.mapper.clone();
        let reconnection = self.reconnection.clone();
        let connected = self.connected.clone();
        connected.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            while let Some(message) = msg_rx.recv().await {
                if message.is_binary {
                    match protocol::decode_packet(&message.payload) {
                        Ok(packet) => {
                            if let FeedPacket::Disconnect { code } = packet {
                                warn!("Dhan feed disconnected by server: code {code}");
                                connected.store(false, Ordering::SeqCst);
                                reconnection
                                    .set_status(
                                        tantra_market_core::ConnectionStatus::Disconnected {
                                            reason: format!("server disconnect code {code}"),
                                        },
                                    )
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
        self.reconnection
            .set_status(tantra_market_core::ConnectionStatus::Connected)
            .await;
        Ok(())
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
    async fn security_id(&self, instrument: &InstrumentId) -> Result<String, MdError> {
        if let Some(token) = &instrument.token {
            return Ok(token.clone());
        }
        if let Some(key) = self.mapper.read().await.to_broker_key(instrument) {
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
    Price::new(Decimal::from_f64_retain(value).unwrap_or(Decimal::ZERO))
}

/// Converts an integral quantity into an exact `Decimal` `Quantity`.
fn to_quantity(value: i64) -> Quantity {
    Quantity::new(Decimal::from(value))
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
        // Group by mode; each mode maps to its own subscribe request code.
        let mut by_mode: Vec<(Mode, Vec<DhanInstrument>)> = Vec::new();
        for sub in subs {
            let segment = Self::exchange_segment(sub.instrument.exchange)?;
            let security_id = self.security_id(&sub.instrument).await?;
            {
                let mut mapper = self.mapper.write().await;
                mapper.register(
                    format!("{}|{security_id}", segment.code()),
                    sub.instrument.clone(),
                );
            }
            self.reconnection.add_subscription(sub.clone()).await;
            if let Some((_, list)) = by_mode.iter_mut().find(|(m, _)| *m == sub.mode) {
                list.push(DhanInstrument {
                    exchange_segment: segment.name(),
                    security_id: security_id.clone(),
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
            let request_code = Self::request_code(mode);
            for chunk in instruments.chunks(protocol::MAX_INSTRUMENTS_PER_MESSAGE) {
                let json = subscribe_json(request_code, chunk)
                    .map_err(|e| MdError::SubscriptionFailed(e.to_string()))?;
                self.ws_client
                    .send_text(json)
                    .await
                    .map_err(|e| MdError::Transport(e.to_string()))?;
            }
        }

        let stream = BroadcastStream::new(self.ticks.subscribe()).filter_map(|result| result.ok());
        Ok(Box::pin(stream))
    }

    async fn unsubscribe(&self, instruments: &[InstrumentId]) -> Result<(), MdError> {
        for instrument in instruments {
            // ReconnectionManager tracks the active subscription set; find this
            // instrument's mode to pick the right unsubscribe request code.
            let resubscribe = self.reconnection.get_resubscribe_list().await;
            let active = resubscribe
                .iter()
                .find(|s| &s.instrument == instrument)
                .cloned();
            let Some(subscription) = active else {
                continue;
            };
            let segment = Self::exchange_segment(subscription.instrument.exchange)?;
            let security_id = self.security_id(&subscription.instrument).await?;
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
        // DhanHQ v2 REST market-quote (`POST /v2/marketfeed/ltp`) — used for
        // reconciliation after reconnection. Body: { segment_name: [security_id, ...] }.
        let mut body: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for instrument in instruments {
            let segment = Self::exchange_segment(instrument.exchange)?;
            let security_id = self.security_id(instrument).await?;
            body.entry(segment.name().to_string())
                .or_default()
                .push(security_id);
        }
        if body.is_empty() {
            return Ok(Vec::new());
        }
        let client = reqwest::Client::new();
        let response = client
            .post(MARKET_QUOTE_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("access-token", &self.access_token)
            .header("client-id", &self.client_id)
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
        let mapper = self.mapper.read().await;
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
                let Some(instrument) = mapper.to_canonical(&key) else {
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
