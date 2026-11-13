//! DhanHQ Live Market Feed v2 wire protocol.
//!
//! Decoders for the binary response packets and builders for the JSON request
//! messages, transcribed from the official DhanHQ v2 Live Market Feed
//! documentation (`https://dhanhq.co/docs/v2/live-market-feed`) and its annexure
//! (`.../annexure`, exchange segments + feed request/response codes).
//!
//! All multi-byte fields are **little-endian**. Every response carries the 8-byte
//! [`ResponseHeader`] followed by a payload whose layout depends on the response code:
//!
//! | Code | Packet | Length | Payload |
//! |------|--------|-------:|---------|
//! | 2    | Ticker | 16 | `f32 ltp`, `i32 last_trade_time` |
//! | 4    | Quote  | 50 | `f32 ltp`, `i16 ltq`, `i32 ltt`, `f32 atp`, `i32 volume`, `i32 sell`, `i32 buy`, `f32 open`, `f32 close`, `f32 high`, `f32 low` |
//! | 5    | OI     | 12 | `i32 open_interest` |
//! | 6    | Prev close | 16 | `f32 prev_close`, `i32 prev_oi` |
//! | 8    | Full   | 162 | quote payload + `i32 oi`, `i32 oi_day_high`, `i32 oi_day_low` + 5×20-byte depth levels |
//! | 50   | Disconnect | 10 | `i16 code` |
//!
//! Prices arrive as `f32` floats on the wire (a documented exception to the zero-floats
//! policy, confined to this adapter's boundary): decoders surface them as `f64` and the
//! feed converts them to exact `Decimal` before they reach any canonical `Tick`.

use thiserror::Error;

/// Feed request codes (DhanHQ annexure, "Feed Request Code").
pub mod request {
    /// Connect the feed.
    pub const CONNECT: i32 = 11;
    /// Disconnect the feed.
    pub const DISCONNECT: i32 = 12;
    /// Subscribe — ticker (LTP) packet.
    pub const SUBSCRIBE_TICKER: i32 = 15;
    /// Unsubscribe — ticker packet.
    pub const UNSUBSCRIBE_TICKER: i32 = 16;
    /// Subscribe — quote packet.
    pub const SUBSCRIBE_QUOTE: i32 = 17;
    /// Unsubscribe — quote packet.
    pub const UNSUBSCRIBE_QUOTE: i32 = 18;
    /// Subscribe — full packet (quote + OI + market depth).
    pub const SUBSCRIBE_FULL: i32 = 21;
    /// Unsubscribe — full packet.
    pub const UNSUBSCRIBE_FULL: i32 = 22;
}

/// Feed response codes (DhanHQ annexure, "Feed Response Code").
pub mod response {
    /// Index packet.
    pub const INDEX: u8 = 1;
    /// Ticker packet.
    pub const TICKER: u8 = 2;
    /// Quote packet.
    pub const QUOTE: u8 = 4;
    /// Open-interest packet.
    pub const OPEN_INTEREST: u8 = 5;
    /// Previous-close packet.
    pub const PREV_CLOSE: u8 = 6;
    /// Market-status packet.
    pub const MARKET_STATUS: u8 = 7;
    /// Full packet.
    pub const FULL: u8 = 8;
    /// Feed-disconnect packet.
    pub const DISCONNECT: u8 = 50;
}

/// Maximum instruments per JSON subscribe message.
pub const MAX_INSTRUMENTS_PER_MESSAGE: usize = 100;

/// Wire-protocol errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// The payload is shorter than the packet layout requires.
    #[error("truncated {packet} packet: need {need} bytes, have {have}")]
    Truncated {
        /// Which packet layout was being decoded.
        packet: &'static str,
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// A subscribe/disconnect message was built with an invalid instrument list.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The JSON request message could not be serialized.
    #[error("request serialization failed: {0}")]
    Json(String),
}

/// A convenience result alias for protocol decoding.
pub type ProtocolResult<T> = Result<T, ProtocolError>;

/// DhanHQ exchange segments (annexure "Exchange Segment").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeSegment {
    /// Index value (IDX_I).
    Index,
    /// NSE equity cash.
    NseEq,
    /// NSE futures & options.
    NseFno,
    /// NSE currency.
    NseCurrency,
    /// BSE equity cash.
    BseEq,
    /// MCX commodity.
    McxComm,
    /// BSE currency.
    BseCurrency,
    /// BSE futures & options.
    BseFno,
}

impl ExchangeSegment {
    /// Maps a wire segment byte to its enum member.
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Index,
            1 => Self::NseEq,
            2 => Self::NseFno,
            3 => Self::NseCurrency,
            4 => Self::BseEq,
            5 => Self::McxComm,
            7 => Self::BseCurrency,
            8 => Self::BseFno,
            _ => return None,
        })
    }

    /// The wire byte for this segment.
    pub fn code(self) -> u8 {
        match self {
            Self::Index => 0,
            Self::NseEq => 1,
            Self::NseFno => 2,
            Self::NseCurrency => 3,
            Self::BseEq => 4,
            Self::McxComm => 5,
            Self::BseCurrency => 7,
            Self::BseFno => 8,
        }
    }

    /// The annexure enum spelling used in JSON requests (`"NSE_EQ"`, ...).
    pub fn name(self) -> &'static str {
        match self {
            Self::Index => "IDX_I",
            Self::NseEq => "NSE_EQ",
            Self::NseFno => "NSE_FNO",
            Self::NseCurrency => "NSE_CURRENCY",
            Self::BseEq => "BSE_EQ",
            Self::McxComm => "MCX_COMM",
            Self::BseCurrency => "BSE_CURRENCY",
            Self::BseFno => "BSE_FNO",
        }
    }

    /// Parses the annexure enum spelling used in JSON (`"NSE_EQ"`, ...).
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "IDX_I" => Self::Index,
            "NSE_EQ" => Self::NseEq,
            "NSE_FNO" => Self::NseFno,
            "NSE_CURRENCY" => Self::NseCurrency,
            "BSE_EQ" => Self::BseEq,
            "MCX_COMM" => Self::McxComm,
            "BSE_CURRENCY" => Self::BseCurrency,
            "BSE_FNO" => Self::BseFno,
            _ => return None,
        })
    }
}

/// The 8-byte response header shared by every feed packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseHeader {
    /// Feed response code (see [`response`]).
    pub response_code: u8,
    /// Message length of the entire payload packet.
    pub message_length: u16,
    /// Exchange segment of the instrument.
    pub exchange_segment: u8,
    /// Exchange security id of the instrument.
    pub security_id: u32,
}

/// Ticker packet (response code 2): last traded price + last trade time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickerData {
    /// Last traded price.
    pub last_price: f64,
    /// Last trade time as a unix epoch (seconds).
    pub last_trade_time: i64,
}

/// Previous-close packet (response code 6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrevCloseData {
    /// Previous day closing price.
    pub prev_close: f64,
    /// Previous day open interest.
    pub prev_oi: i64,
}

/// Quote packet (response code 4): trade + volume + day OHLC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuoteData {
    /// Last traded price.
    pub last_price: f64,
    /// Last traded quantity.
    pub last_traded_qty: i16,
    /// Last trade time as a unix epoch (seconds).
    pub last_trade_time: i64,
    /// Volume-weighted average trade price.
    pub average_trade_price: f64,
    /// Total traded volume for the day.
    pub volume: i64,
    /// Total sell quantity pending at the exchange.
    pub total_sell_qty: i64,
    /// Total buy quantity pending at the exchange.
    pub total_buy_qty: i64,
    /// Day open.
    pub day_open: f64,
    /// Day close (sent post market close).
    pub day_close: f64,
    /// Day high.
    pub day_high: f64,
    /// Day low.
    pub day_low: f64,
}

/// Open-interest packet (response code 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OiData {
    /// Open interest of the contract.
    pub open_interest: i64,
}

/// One market-depth level in a full packet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthLevel {
    /// Bid quantity.
    pub bid_qty: i64,
    /// Ask quantity.
    pub ask_qty: i64,
    /// Number of open bid orders.
    pub bid_orders: i16,
    /// Number of open ask orders.
    pub ask_orders: i16,
    /// Bid price.
    pub bid_price: f64,
    /// Ask price.
    pub ask_price: f64,
}

/// Full packet (response code 8): quote + OI + five depth levels.
#[derive(Debug, Clone, PartialEq)]
pub struct FullData {
    /// The quote portion of the packet.
    pub quote: QuoteData,
    /// Open interest of the contract (derivatives).
    pub open_interest: i64,
    /// Highest open interest of the day (NSE_FNO only).
    pub oi_day_high: i64,
    /// Lowest open interest of the day (NSE_FNO only).
    pub oi_day_low: i64,
    /// Five market-depth levels, best first.
    pub depth: [DepthLevel; 5],
}

/// A decoded feed packet.
#[derive(Debug, Clone, PartialEq)]
pub enum FeedPacket {
    /// Ticker packet (code 2).
    Ticker(ResponseHeader, TickerData),
    /// Quote packet (code 4).
    Quote(ResponseHeader, QuoteData),
    /// Open-interest packet (code 5).
    OpenInterest(ResponseHeader, OiData),
    /// Previous-close packet (code 6).
    PrevClose(ResponseHeader, PrevCloseData),
    /// Full packet (code 8) — boxed to keep the enum small (it carries five depth levels).
    Full(ResponseHeader, Box<FullData>),
    /// Feed-disconnect packet (code 50).
    Disconnect { code: i16 },
    /// A packet whose response code this decoder does not decode yet.
    Unknown { code: u8 },
}

/// Decodes one binary feed message into a [`FeedPacket`].
pub fn decode_packet(bytes: &[u8]) -> ProtocolResult<FeedPacket> {
    let header = decode_header(bytes)?;
    match header.response_code {
        response::TICKER => Ok(FeedPacket::Ticker(
            header,
            TickerData {
                last_price: decode_f32(bytes, 8, "ticker")?,
                last_trade_time: i64::from(decode_i32(bytes, 12, "ticker")?),
            },
        )),
        response::QUOTE => Ok(FeedPacket::Quote(header, decode_quote(bytes, 8)?)),
        response::OPEN_INTEREST => Ok(FeedPacket::OpenInterest(
            header,
            OiData {
                open_interest: i64::from(decode_i32(bytes, 8, "open_interest")?),
            },
        )),
        response::PREV_CLOSE => Ok(FeedPacket::PrevClose(
            header,
            PrevCloseData {
                prev_close: decode_f32(bytes, 8, "prev_close")?,
                prev_oi: i64::from(decode_i32(bytes, 12, "prev_close")?),
            },
        )),
        response::FULL => {
            let quote = decode_quote(bytes, 8)?;
            let open_interest = i64::from(decode_i32(bytes, 34, "full")?);
            let oi_day_high = i64::from(decode_i32(bytes, 38, "full")?);
            let oi_day_low = i64::from(decode_i32(bytes, 42, "full")?);
            let mut depth = [DepthLevel {
                bid_qty: 0,
                ask_qty: 0,
                bid_orders: 0,
                ask_orders: 0,
                bid_price: 0.0,
                ask_price: 0.0,
            }; 5];
            for (level, slot) in depth.iter_mut().enumerate() {
                let base = 62 + level * 20;
                if bytes.len() < base + 20 {
                    return Err(ProtocolError::Truncated {
                        packet: "full",
                        need: base + 20,
                        have: bytes.len(),
                    });
                }
                *slot = DepthLevel {
                    bid_qty: i64::from(i32::from_le_bytes(
                        bytes[base..base + 4].try_into().unwrap(),
                    )),
                    ask_qty: i64::from(i32::from_le_bytes(
                        bytes[base + 4..base + 8].try_into().unwrap(),
                    )),
                    bid_orders: i16::from_le_bytes(bytes[base + 8..base + 10].try_into().unwrap()),
                    ask_orders: i16::from_le_bytes(bytes[base + 10..base + 12].try_into().unwrap()),
                    bid_price: f64::from(f32::from_le_bytes(
                        bytes[base + 12..base + 16].try_into().unwrap(),
                    )),
                    ask_price: f64::from(f32::from_le_bytes(
                        bytes[base + 16..base + 20].try_into().unwrap(),
                    )),
                };
            }
            Ok(FeedPacket::Full(
                header,
                Box::new(FullData {
                    quote,
                    open_interest,
                    oi_day_high,
                    oi_day_low,
                    depth,
                }),
            ))
        }
        response::DISCONNECT => {
            if bytes.len() < 10 {
                return Err(ProtocolError::Truncated {
                    packet: "disconnect",
                    need: 10,
                    have: bytes.len(),
                });
            }
            Ok(FeedPacket::Disconnect {
                code: i16::from_le_bytes(bytes[8..10].try_into().unwrap()),
            })
        }
        code => Ok(FeedPacket::Unknown { code }),
    }
}

/// The quote-payload portion of a quote (code 4) or full (code 8) packet.
fn decode_quote(bytes: &[u8], base: usize) -> ProtocolResult<QuoteData> {
    if bytes.len() < base + 42 {
        return Err(ProtocolError::Truncated {
            packet: "quote",
            need: base + 42,
            have: bytes.len(),
        });
    }
    Ok(QuoteData {
        last_price: f64::from(f32::from_le_bytes(
            bytes[base..base + 4].try_into().unwrap(),
        )),
        last_traded_qty: i16::from_le_bytes(bytes[base + 4..base + 6].try_into().unwrap()),
        last_trade_time: i64::from(i32::from_le_bytes(
            bytes[base + 6..base + 10].try_into().unwrap(),
        )),
        average_trade_price: f64::from(f32::from_le_bytes(
            bytes[base + 10..base + 14].try_into().unwrap(),
        )),
        volume: i64::from(i32::from_le_bytes(
            bytes[base + 14..base + 18].try_into().unwrap(),
        )),
        total_sell_qty: i64::from(i32::from_le_bytes(
            bytes[base + 18..base + 22].try_into().unwrap(),
        )),
        total_buy_qty: i64::from(i32::from_le_bytes(
            bytes[base + 22..base + 26].try_into().unwrap(),
        )),
        day_open: f64::from(f32::from_le_bytes(
            bytes[base + 26..base + 30].try_into().unwrap(),
        )),
        day_close: f64::from(f32::from_le_bytes(
            bytes[base + 30..base + 34].try_into().unwrap(),
        )),
        day_high: f64::from(f32::from_le_bytes(
            bytes[base + 34..base + 38].try_into().unwrap(),
        )),
        day_low: f64::from(f32::from_le_bytes(
            bytes[base + 38..base + 42].try_into().unwrap(),
        )),
    })
}

fn decode_header(bytes: &[u8]) -> ProtocolResult<ResponseHeader> {
    if bytes.len() < 8 {
        return Err(ProtocolError::Truncated {
            packet: "header",
            need: 8,
            have: bytes.len(),
        });
    }
    Ok(ResponseHeader {
        response_code: bytes[0],
        message_length: u16::from_le_bytes(bytes[1..3].try_into().unwrap()),
        exchange_segment: bytes[3],
        security_id: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
    })
}

fn decode_f32(bytes: &[u8], at: usize, packet: &'static str) -> ProtocolResult<f64> {
    if bytes.len() < at + 4 {
        return Err(ProtocolError::Truncated {
            packet,
            need: at + 4,
            have: bytes.len(),
        });
    }
    Ok(f64::from(f32::from_le_bytes(
        bytes[at..at + 4].try_into().unwrap(),
    )))
}

fn decode_i32(bytes: &[u8], at: usize, packet: &'static str) -> ProtocolResult<i32> {
    if bytes.len() < at + 4 {
        return Err(ProtocolError::Truncated {
            packet,
            need: at + 4,
            have: bytes.len(),
        });
    }
    Ok(i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()))
}

/// One instrument in a subscribe/disconnect JSON request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DhanInstrument {
    /// Exchange segment enum spelling (e.g. `"NSE_EQ"`).
    #[serde(rename = "ExchangeSegment")]
    pub exchange_segment: &'static str,
    /// Exchange security id as a string.
    #[serde(rename = "SecurityId")]
    pub security_id: String,
}

/// Builds the JSON subscribe message for `request_code` and `instruments`
/// (max [`MAX_INSTRUMENTS_PER_MESSAGE`] per message; send more messages for more).
pub fn subscribe_json(request_code: i32, instruments: &[DhanInstrument]) -> ProtocolResult<String> {
    if instruments.is_empty() {
        return Err(ProtocolError::InvalidRequest(
            "subscribe request needs at least one instrument".into(),
        ));
    }
    if instruments.len() > MAX_INSTRUMENTS_PER_MESSAGE {
        return Err(ProtocolError::InvalidRequest(format!(
            "subscribe request carries {} instruments; Dhan allows at most {MAX_INSTRUMENTS_PER_MESSAGE} per message",
            instruments.len()
        )));
    }
    #[derive(serde::Serialize)]
    struct Message<'a> {
        #[serde(rename = "RequestCode")]
        request_code: i32,
        #[serde(rename = "InstrumentCount")]
        instrument_count: usize,
        #[serde(rename = "InstrumentList")]
        instrument_list: &'a [DhanInstrument],
    }
    serde_json::to_string(&Message {
        request_code,
        instrument_count: instruments.len(),
        instrument_list: instruments,
    })
    .map_err(|e| ProtocolError::Json(e.to_string()))
}

/// Builds the JSON disconnect message.
pub fn disconnect_json() -> String {
    r#"{"RequestCode":12}"#.to_string()
}
