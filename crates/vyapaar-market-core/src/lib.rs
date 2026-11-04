//! # व्यापार・बाज़ार vyapaar-market-core
//!
//! Canonical market data feed interfaces, broker symbol mapping, and
//! microstructure enrichment for Indian equities.
//!
//! This crate is deliberately dependency-light and WASM-portable: it is
//! 100% synchronous with no `tokio`, no OS filesystem I/O and no sockets.
//! The async runtime components of the source crate (`CoalescingTickBuffer`,
//! `ReconnectionManager`, and the shared `WsClient` transport) live in
//! [`vyapaar_market_feed`] (tokens: `https://docs.rs/vyapaar-market-feed`),
//! which is the only place `tokio` is allowed.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use rust_decimal::Decimal;
use std::collections::HashMap;
use thiserror::Error;

pub use vyapaar_core::{Bar, DepthLevel, Exchange, InstrumentId, Price, PriceBand, Quantity, Tick};

/// Subscription depth/mode for market data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Last Traded Price only (lightweight)
    Ltp,
    /// Best bid/ask quote with scrip price band
    Quote,
    /// Full market depth (typically top 5 levels in NSE/BSE)
    Full,
}

/// A market data subscription request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Subscription {
    /// Instrument to stream
    pub instrument: InstrumentId,
    /// Mode requested
    pub mode: Mode,
}

impl Subscription {
    /// Create a new subscription in specified mode.
    pub fn new(instrument: InstrumentId, mode: Mode) -> Self {
        Self { instrument, mode }
    }
}

/// Errors occurring in market data ingestion.
#[derive(Debug, Error)]
pub enum MdError {
    /// Broker authentication or session failure.
    #[error("market data auth failure: {0}")]
    Auth(String),

    /// Network or WebSocket transport error.
    #[error("market data transport error: {0}")]
    Transport(String),

    /// Subscription rejected by broker.
    #[error("subscription error: {0}")]
    SubscriptionFailed(String),

    /// Buffer full and non-coalesceable tick was dropped.
    #[error("backpressure drop: buffer capacity exceeded")]
    BackpressureDrop,

    /// Stream timeout.
    #[error("market data stream timeout")]
    Timeout,
}

/// Feed health status with heartbeat and latency metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedHealth {
    /// Provider name.
    pub name: String,
    /// Whether the feed is currently connected.
    pub connected: bool,
    /// Last heartbeat timestamp (if available).
    pub last_heartbeat: Option<DateTime<Utc>>,
    /// Round-trip latency in milliseconds (if measured).
    pub rtt_ms: Option<u64>,
    /// Number of dropped packets/messages.
    pub dropped_count: u64,
    /// Number of reconnection attempts.
    pub reconnect_count: u64,
}

/// Core trait implemented by market data adapters (Zerodha WebSocket, Upstox Feed, Fyers Feed).
#[async_trait]
pub trait MarketDataFeed: Send + Sync {
    /// Feed provider name (e.g., "zerodha", "upstox", "fyers").
    fn name(&self) -> &str;

    /// Subscribe to a set of instruments and return a normalized stream of canonical [`Tick`]s.
    async fn subscribe(&self, subs: Vec<Subscription>)
    -> Result<BoxStream<'static, Tick>, MdError>;

    /// Unsubscribe from streaming market data.
    async fn unsubscribe(&self, instruments: &[InstrumentId]) -> Result<(), MdError>;

    /// Fetch a snapshot of current quotes for the given instruments via REST.
    /// Used for reconciliation after reconnection to bridge any missed ticks.
    async fn fetch_snapshot(&self, instruments: &[InstrumentId]) -> Result<Vec<Tick>, MdError>;

    /// Get current feed health status.
    fn feed_health(&self) -> FeedHealth;
}

/// Bidirectional symbol mapper translating between broker-specific tokens/tickers
/// and canonical [`InstrumentId`]s.
#[derive(Debug, Default, Clone)]
pub struct SymbolMapper {
    broker_to_canonical: HashMap<String, InstrumentId>,
    canonical_to_broker: HashMap<InstrumentId, String>,
}

impl SymbolMapper {
    /// Create an empty symbol mapper.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a mapping between a broker identifier and canonical instrument.
    pub fn register(&mut self, broker_key: impl Into<String>, canonical: InstrumentId) {
        let key = broker_key.into();
        self.canonical_to_broker
            .insert(canonical.clone(), key.clone());
        self.broker_to_canonical.insert(key, canonical);
    }

    /// Resolve a broker key or token into a canonical InstrumentId.
    ///
    /// If not registered in the dictionary, applies standard heuristic parsing for Indian brokers:
    /// - `NSE:INFY` -> Exchange::Nse, "INFY"
    /// - `NSE_EQ|INE009A01021` (Upstox) -> Exchange::Nse, "INE009A01021"
    /// - `INFY$NSE` -> Exchange::Nse, "INFY"
    /// - `NSE:INFY-EQ` (Fyers) -> Exchange::Nse, "INFY"
    pub fn to_canonical(&self, broker_key: &str) -> Option<InstrumentId> {
        if let Some(inst) = self.broker_to_canonical.get(broker_key) {
            return Some(inst.clone());
        }

        // Heuristic parsing
        Self::parse_heuristic(broker_key)
    }

    /// Resolve a canonical InstrumentId to its registered broker key.
    pub fn to_broker_key(&self, inst: &InstrumentId) -> Option<&str> {
        self.canonical_to_broker.get(inst).map(|s| s.as_str())
    }

    fn parse_heuristic(s: &str) -> Option<InstrumentId> {
        // Pattern 1: INFY$NSE
        if let Some((sym, exch)) = s.split_once('$') {
            let exchange = exch.parse().ok()?;
            return Some(InstrumentId::new(exchange, sym));
        }

        // Pattern 2: NSE:INFY-EQ or NSE:INFY
        if let Some((exch, sym_part)) = s.split_once(':') {
            let exchange = exch.parse().ok()?;
            let sym = sym_part.trim_end_matches("-EQ");
            return Some(InstrumentId::new(exchange, sym));
        }

        // Pattern 3: NSE_EQ|INE009A01021
        if let Some((exch_part, token_or_sym)) = s.split_once('|') {
            let exch = exch_part.split('_').next()?;
            let exchange = exch.parse().ok()?;
            return Some(
                InstrumentId::new(exchange, token_or_sym).with_token(token_or_sym.to_string()),
            );
        }

        None
    }
}

/// Scrip price band and microstructure enrichment for market data ticks.
///
/// Wraps incoming raw broker ticks and enriches them with:
/// - Dynamic scrip price bands (queried from `vyapaar-market-normalizer` / daily Bhavcopy).
/// - Prev close, 52-week high/low, and day open.
/// - Monotonically calculated intraday VWAP.
#[derive(Debug, Clone, Default)]
pub struct MarketDataEnricher {
    /// Per-scrip price bands.
    price_bands: HashMap<InstrumentId, PriceBand>,
    /// Per-scrip previous close price.
    prev_close: HashMap<InstrumentId, Price>,
    /// Per-scrip VWAP state (cumulative price*volume and cumulative volume).
    vwap_state: HashMap<InstrumentId, (Price, Quantity)>,
}

impl MarketDataEnricher {
    /// Create a new enricher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the price band for a scrip.
    pub fn set_price_band(&mut self, instrument: InstrumentId, band: PriceBand) {
        self.price_bands.insert(instrument, band);
    }

    /// Set the previous close price for a scrip.
    pub fn set_prev_close(&mut self, instrument: InstrumentId, price: Price) {
        self.prev_close.insert(instrument, price);
    }

    /// Enrich a tick with price band and VWAP information.
    ///
    /// For `Tick::Quote` variants, attaches the scrip's price band if available.
    /// For all tick types, updates the intraday VWAP state using the last price
    /// and available volume.
    pub fn enrich(&mut self, tick: &mut Tick) {
        let inst = tick.instrument().clone();

        // Attach price band if available
        if let Tick::Quote { band, .. } = tick
            && band.is_none()
            && let Some(b) = self.price_bands.get(&inst)
        {
            *band = Some(*b);
        }

        // Update VWAP state from tick price and volume
        let (price, volume) = match tick {
            Tick::Ltp { price, volume, .. } => (*price, *volume),
            Tick::Quote {
                bid, ask, bid_qty, ..
            } => {
                let mid = Price((bid.0 + ask.0) / Decimal::from(2));
                let vol = *bid_qty;
                (mid, vol)
            }
            Tick::Depth { last_price, .. } => {
                let p = match *last_price {
                    Some(p) => p,
                    None => Price::ZERO,
                };
                (p, Quantity::ZERO)
            }
        };

        if volume.0 > Decimal::ZERO {
            let state = self.vwap_state.entry(inst).or_default();
            state.0 = Price(state.0.0 + price.0 * volume.0);
            state.1 = Quantity(state.1.0 + volume.0);
        }
    }

    /// Get the current VWAP for a scrip, if any data has been seen.
    pub fn vwap(&self, instrument: &InstrumentId) -> Option<Price> {
        self.vwap_state.get(instrument).and_then(|(pv, v)| {
            if v.0 > Decimal::ZERO {
                Some(Price(pv.0 / v.0))
            } else {
                None
            }
        })
    }
}
