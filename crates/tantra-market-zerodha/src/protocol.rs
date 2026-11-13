//! Zerodha Kite Connect 3 WebSocket binary wire protocol.
//!
//! Transcribed from the official Kite Connect 3 WebSocket docs
//! (`https://kite.trade/docs/connect/v3/websocket`). A WebSocket **message** frames one or
//! more quote **packets**: a little-endian `i16` packet count, then per packet a little
//! endian `i16` byte length followed by the packet bytes. Packet lengths are mode- and
//! index-determinate, so the decoder dispatches without extra metadata:
//!
//! | Mode  | Instrument | Index |
//! |-------|-----------:|------:|
//! | ltp   | 8  | 8   |
//! | quote | 44 | 28  |
//! | full  | 184| 32  |
//!
//! All prices are integer **paise** (Kite divides by 100 for equity/derivatives, by
//! 10⁷ for currency); the feed converts them to exact `Decimal` rupees at the boundary.

use thiserror::Error;

/// Request actions (the JSON `a` field).
pub mod action {
    /// Subscribe to tokens.
    pub const SUBSCRIBE: &str = "subscribe";
    /// Unsubscribe from tokens.
    pub const UNSUBSCRIBE: &str = "unsubscribe";
    /// Set the stream mode for tokens.
    pub const MODE: &str = "mode";
}

/// Packet lengths in bytes.
pub mod packet_len {
    /// LTP-only instrument/index packet.
    pub const LTP: usize = 8;
    /// Quote instrument packet (no market depth).
    pub const INSTRUMENT_QUOTE: usize = 44;
    /// Quote index packet.
    pub const INDEX_QUOTE: usize = 28;
    /// Full instrument packet (quote + depth).
    pub const INSTRUMENT_FULL: usize = 184;
    /// Full index packet.
    pub const INDEX_FULL: usize = 32;
}

/// Maximum packets allowed in one message frame (guards against absurd allocations).
pub const MAX_PACKETS_PER_MESSAGE: usize = 3000;

/// Wire-protocol errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// The frame is shorter than its packet count claims, or a packet length overruns.
    #[error("malformed frame: {0}")]
    Malformed(String),
    /// A packet's byte length does not match any known layout for the mode.
    #[error("unexpected {mode} packet length {got}; expected one of {expected}")]
    UnexpectedLength {
        /// The subscription mode the packet arrived under.
        mode: &'static str,
        /// The packet's actual byte length.
        got: usize,
        /// The accepted lengths for that mode.
        expected: &'static str,
    },
}

/// A convenience result alias for protocol decoding.
pub type ProtocolResult<T> = Result<T, ProtocolError>;

/// One quote packet extracted from a message frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    /// The packet's byte length.
    pub len: usize,
    /// The instrument token.
    pub token: u32,
    /// The raw packet payload (from the token field onward).
    pub payload: Vec<u8>,
}

/// A fully decoded quote packet for a tradeable instrument (all prices in paise).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstrumentQuote {
    /// Instrument token.
    pub token: u32,
    /// Last traded price (paise).
    pub last_price: i32,
    /// Last traded quantity.
    pub last_traded_qty: i32,
    /// Average traded price (paise).
    pub average_traded_price: i32,
    /// Volume traded for the day.
    pub volume: i32,
    /// Total buy quantity.
    pub total_buy_qty: i32,
    /// Total sell quantity.
    pub total_sell_qty: i32,
    /// Day open (paise).
    pub open: i32,
    /// Day high (paise).
    pub high: i32,
    /// Day low (paise).
    pub low: i32,
    /// Day close (paise).
    pub close: i32,
    /// Last traded timestamp (unix seconds); full mode only.
    pub last_traded_timestamp: Option<i32>,
    /// Open interest (full mode only).
    pub open_interest: Option<i32>,
    /// OI day high (full mode only).
    pub oi_day_high: Option<i32>,
    /// OI day low (full mode only).
    pub oi_day_low: Option<i32>,
    /// Exchange timestamp (full mode only).
    pub exchange_timestamp: Option<i32>,
    /// Ten market-depth entries, five bids then five asks (full mode only).
    pub depth: Option<[DepthEntry; 10]>,
}

/// One market-depth entry (quantity, price, orders, plus 2 bytes padding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthEntry {
    /// Depth quantity.
    pub quantity: i32,
    /// Depth price (paise).
    pub price: i32,
    /// Number of open orders.
    pub orders: i16,
}

/// A fully decoded quote packet for an index (all prices in paise).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndexQuote {
    /// Instrument token.
    pub token: u32,
    /// Last traded price (paise).
    pub last_price: i32,
    /// Day high (paise).
    pub high: i32,
    /// Day low (paise).
    pub low: i32,
    /// Day open (paise).
    pub open: i32,
    /// Day close (paise).
    pub close: i32,
    /// Absolute change (paise); quote mode only.
    pub price_change: Option<i32>,
    /// Exchange timestamp; full mode only.
    pub exchange_timestamp: Option<i32>,
}

/// A decoded feed packet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FeedPacket {
    /// LTP-only packet (mode `ltp`, 8 bytes — shared instrument/index layout).
    Ltp {
        /// Instrument token.
        token: u32,
        /// Last traded price (paise).
        last_price: i32,
    },
    /// Instrument quote packet (44 or 184 bytes).
    Instrument(InstrumentQuote),
    /// Index quote packet (28 or 32 bytes).
    Index(IndexQuote),
}

/// Splits a binary message frame into its individual quote packets.
pub fn decode_message(bytes: &[u8]) -> ProtocolResult<Vec<Packet>> {
    if bytes.len() < 2 {
        return Err(ProtocolError::Malformed(format!(
            "frame shorter than the 2-byte packet count ({} bytes)",
            bytes.len()
        )));
    }
    let count = i16::from_le_bytes([bytes[0], bytes[1]]);
    if count < 0 {
        return Err(ProtocolError::Malformed(format!(
            "negative packet count {count}"
        )));
    }
    let count = count as usize;
    if count > MAX_PACKETS_PER_MESSAGE {
        return Err(ProtocolError::Malformed(format!(
            "packet count {count} exceeds the {MAX_PACKETS_PER_MESSAGE} cap"
        )));
    }
    let mut packets = Vec::with_capacity(count);
    let mut at = 2;
    for _ in 0..count {
        if bytes.len() < at + 2 {
            return Err(ProtocolError::Malformed(format!(
                "packet {} length field overruns the frame",
                packets.len()
            )));
        }
        let len = i16::from_le_bytes([bytes[at], bytes[at + 1]]);
        if len < 0 {
            return Err(ProtocolError::Malformed(format!(
                "negative packet length {len}"
            )));
        }
        let len = len as usize;
        at += 2;
        if bytes.len() < at + len {
            return Err(ProtocolError::Malformed(format!(
                "packet {} of length {len} overruns the frame",
                packets.len()
            )));
        }
        let packet = &bytes[at..at + len];
        if packet.len() < 4 {
            return Err(ProtocolError::Malformed(format!(
                "packet {} is shorter than its token field",
                packets.len()
            )));
        }
        packets.push(Packet {
            len,
            token: u32::from_le_bytes(packet[0..4].try_into().unwrap()),
            payload: packet.to_vec(),
        });
        at += len;
    }
    Ok(packets)
}

/// Decodes one quote packet for a subscription `mode`.
pub fn decode_packet(
    mode: tantra_market_core::Mode,
    packet: &Packet,
) -> ProtocolResult<FeedPacket> {
    match mode {
        tantra_market_core::Mode::Ltp => {
            if packet.payload.len() != packet_len::LTP {
                return Err(unexpected_length("ltp", packet.payload.len(), "8"));
            }
            Ok(FeedPacket::Ltp {
                token: packet.token,
                last_price: i32_at(&packet.payload, 4)?,
            })
        }
        tantra_market_core::Mode::Quote => match packet.payload.len() {
            packet_len::INSTRUMENT_QUOTE => Ok(FeedPacket::Instrument(decode_instrument(
                &packet.payload,
                false,
            )?)),
            packet_len::INDEX_QUOTE => Ok(FeedPacket::Index(decode_index(&packet.payload, false)?)),
            got => Err(unexpected_length(
                "quote",
                got,
                "44 (instrument) or 28 (index)",
            )),
        },
        tantra_market_core::Mode::Full => match packet.payload.len() {
            packet_len::INSTRUMENT_FULL => Ok(FeedPacket::Instrument(decode_instrument(
                &packet.payload,
                true,
            )?)),
            packet_len::INDEX_FULL => Ok(FeedPacket::Index(decode_index(&packet.payload, true)?)),
            got => Err(unexpected_length(
                "full",
                got,
                "184 (instrument) or 32 (index)",
            )),
        },
    }
}

/// Decodes an instrument quote packet (44 bytes, or 184 in full mode).
fn decode_instrument(bytes: &[u8], full: bool) -> ProtocolResult<InstrumentQuote> {
    let need = if full {
        packet_len::INSTRUMENT_FULL
    } else {
        packet_len::INSTRUMENT_QUOTE
    };
    if bytes.len() != need {
        return Err(unexpected_length("quote", bytes.len(), "44 or 184"));
    }
    let mut quote = InstrumentQuote {
        token: u32_at(bytes, 0)?,
        last_price: i32_at(bytes, 4)?,
        last_traded_qty: i32_at(bytes, 8)?,
        average_traded_price: i32_at(bytes, 12)?,
        volume: i32_at(bytes, 16)?,
        total_buy_qty: i32_at(bytes, 20)?,
        total_sell_qty: i32_at(bytes, 24)?,
        open: i32_at(bytes, 28)?,
        high: i32_at(bytes, 32)?,
        low: i32_at(bytes, 36)?,
        close: i32_at(bytes, 40)?,
        last_traded_timestamp: None,
        open_interest: None,
        oi_day_high: None,
        oi_day_low: None,
        exchange_timestamp: None,
        depth: None,
    };
    if full {
        quote.last_traded_timestamp = Some(i32_at(bytes, 44)?);
        quote.open_interest = Some(i32_at(bytes, 48)?);
        quote.oi_day_high = Some(i32_at(bytes, 52)?);
        quote.oi_day_low = Some(i32_at(bytes, 56)?);
        quote.exchange_timestamp = Some(i32_at(bytes, 60)?);
        let mut depth = [DepthEntry {
            quantity: 0,
            price: 0,
            orders: 0,
        }; 10];
        for (entry, slot) in depth.iter_mut().enumerate() {
            let base = 64 + entry * 12;
            *slot = DepthEntry {
                quantity: i32_at(bytes, base)?,
                price: i32_at(bytes, base + 4)?,
                orders: i16::from_le_bytes(bytes[base + 8..base + 10].try_into().unwrap()),
            };
            // bytes base+10..base+12 are padding (2 bytes), skipped.
        }
        quote.depth = Some(depth);
    }
    Ok(quote)
}

/// Decodes an index quote packet (28 bytes: token..price change; full mode adds the
/// 32-byte exchange timestamp).
fn decode_index(bytes: &[u8], full: bool) -> ProtocolResult<IndexQuote> {
    let need = if full {
        packet_len::INDEX_FULL
    } else {
        packet_len::INDEX_QUOTE
    };
    if bytes.len() != need {
        return Err(unexpected_length("quote", bytes.len(), "28 or 32"));
    }
    Ok(IndexQuote {
        token: u32_at(bytes, 0)?,
        last_price: i32_at(bytes, 4)?,
        high: i32_at(bytes, 8)?,
        low: i32_at(bytes, 12)?,
        open: i32_at(bytes, 16)?,
        close: i32_at(bytes, 20)?,
        price_change: Some(i32_at(bytes, 24)?),
        exchange_timestamp: if full { Some(i32_at(bytes, 28)?) } else { None },
    })
}

fn unexpected_length(mode: &'static str, got: usize, expected: &'static str) -> ProtocolError {
    ProtocolError::UnexpectedLength {
        mode,
        got,
        expected,
    }
}

fn u32_at(bytes: &[u8], at: usize) -> ProtocolResult<u32> {
    bytes
        .get(at..at + 4)
        .ok_or_else(|| ProtocolError::Malformed(format!("field at {at} overruns packet")))
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn i32_at(bytes: &[u8], at: usize) -> ProtocolResult<i32> {
    bytes
        .get(at..at + 4)
        .ok_or_else(|| ProtocolError::Malformed(format!("field at {at} overruns packet")))
        .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
}

/// Builds a `subscribe`/`unsubscribe` request (`{"a": "subscribe", "v": [...]}`).
pub fn subscribe_json(action: &str, tokens: &[u32]) -> String {
    serde_json::json!({ "a": action, "v": tokens }).to_string()
}

/// Builds a `mode` request (`{"a": "mode", "v": ["ltp", [...]]}`).
pub fn mode_json(mode: &str, tokens: &[u32]) -> String {
    serde_json::json!({ "a": "mode", "v": [mode, tokens] }).to_string()
}
