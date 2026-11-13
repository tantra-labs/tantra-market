//! Golden-byte tests for the DhanHQ v2 tickfeed wire protocol (`docs/gaps/0003`).
//!
//! The byte offsets are transcribed from the official DhanHQ v2 Live Market Feed docs.
//! Vectors are little-endian. One fully hand-written golden vector pins the ticker
//! layout; the rest build distinct-value vectors programmatically so a misplaced offset
//! (which would read another field's bytes) fails the assertion.

use std::sync::Arc;

use chrono::Utc;
use rust_decimal::Decimal;
use tantra_core::{Exchange, InstrumentId, Price, Quantity, Tick};
use tantra_market_core::SymbolMapper;
use tantra_market_dhan::packet_to_tick;
use tantra_market_dhan::protocol::{
    DhanInstrument, ExchangeSegment, FeedPacket, ProtocolError, ResponseHeader, disconnect_json,
    request, subscribe_json,
};
use tokio::sync::RwLock;

/// Header: code 2, message length 16, segment 2 (NSE_FNO), security id 11536 (0x2D10).
const GOLDEN_HEADER: [u8; 8] = [2, 16, 0, 2, 0x10, 0x2D, 0, 0];

fn header(code: u8, length: u16, segment: u8, security_id: u32) -> Vec<u8> {
    let mut bytes = vec![code];
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.push(segment);
    bytes.extend_from_slice(&security_id.to_le_bytes());
    bytes
}

fn f32_le(value: f32) -> [u8; 4] {
    value.to_le_bytes()
}

#[test]
fn golden_ticker_vector_decodes_exactly() {
    // 100.0 f32 = 0x42C80000 (LE 00 00 C8 42); ltt appended programmatically (0 is a
    // trivially-verifiable epoch) so the golden prefix pins the header + LTP offsets.
    let mut bytes = GOLDEN_HEADER.to_vec();
    bytes.extend_from_slice(&[0x00, 0x00, 0xC8, 0x42]);
    bytes.extend_from_slice(&0i32.to_le_bytes());

    let packet = tantra_market_dhan::protocol::decode_packet(&bytes).unwrap();
    match packet {
        FeedPacket::Ticker(header, data) => {
            assert_eq!(
                header,
                ResponseHeader {
                    response_code: 2,
                    message_length: 16,
                    exchange_segment: 2,
                    security_id: 11536,
                }
            );
            assert_eq!(data.last_price, 100.0);
            assert_eq!(data.last_trade_time, 0);
        }
        other => panic!("expected ticker packet, got {other:?}"),
    }
}

#[test]
fn quote_packet_decodes_every_field_at_its_offset() {
    let mut bytes = header(response::QUOTE, 50, 2, 49081);
    bytes.extend_from_slice(&f32_le(100.5)); // ltp
    bytes.extend_from_slice(&25i16.to_le_bytes()); // ltq
    bytes.extend_from_slice(&1_700_000_000i32.to_le_bytes()); // ltt
    bytes.extend_from_slice(&f32_le(99.25)); // atp
    bytes.extend_from_slice(&1_234_567i32.to_le_bytes()); // volume
    bytes.extend_from_slice(&42i32.to_le_bytes()); // sell qty
    bytes.extend_from_slice(&84i32.to_le_bytes()); // buy qty
    bytes.extend_from_slice(&f32_le(97.0)); // day open
    bytes.extend_from_slice(&f32_le(101.0)); // day close
    bytes.extend_from_slice(&f32_le(102.5)); // day high
    bytes.extend_from_slice(&f32_le(96.0)); // day low

    let packet = tantra_market_dhan::protocol::decode_packet(&bytes).unwrap();
    match packet {
        FeedPacket::Quote(header, data) => {
            assert_eq!(header.security_id, 49081);
            assert_eq!(data.last_price, 100.5);
            assert_eq!(data.last_traded_qty, 25);
            assert_eq!(data.last_trade_time, 1_700_000_000);
            assert_eq!(data.average_trade_price, 99.25);
            assert_eq!(data.volume, 1_234_567);
            assert_eq!(data.total_sell_qty, 42);
            assert_eq!(data.total_buy_qty, 84);
            assert_eq!(data.day_open, 97.0);
            assert_eq!(data.day_close, 101.0);
            assert_eq!(data.day_high, 102.5);
            assert_eq!(data.day_low, 96.0);
        }
        other => panic!("expected quote packet, got {other:?}"),
    }
}

#[test]
fn oi_and_prev_close_packets_decode() {
    let mut oi = header(response::OPEN_INTEREST, 12, 2, 49081);
    oi.extend_from_slice(&1_250_000i32.to_le_bytes());
    match tantra_market_dhan::protocol::decode_packet(&oi).unwrap() {
        FeedPacket::OpenInterest(_, data) => assert_eq!(data.open_interest, 1_250_000),
        other => panic!("expected OI packet, got {other:?}"),
    }

    let mut prev = header(response::PREV_CLOSE, 16, 2, 49081);
    prev.extend_from_slice(&f32_le(98.5));
    prev.extend_from_slice(&900_000i32.to_le_bytes());
    match tantra_market_dhan::protocol::decode_packet(&prev).unwrap() {
        FeedPacket::PrevClose(_, data) => {
            assert_eq!(data.prev_close, 98.5);
            assert_eq!(data.prev_oi, 900_000);
        }
        other => panic!("expected prev-close packet, got {other:?}"),
    }
}

#[test]
fn full_packet_decodes_quote_oi_and_five_depth_levels() {
    let mut bytes = header(response::FULL, 162, 2, 49081);
    bytes.extend_from_slice(&f32_le(100.5)); // ltp
    bytes.extend_from_slice(&10i16.to_le_bytes()); // ltq
    bytes.extend_from_slice(&1_700_000_000i32.to_le_bytes()); // ltt
    bytes.extend_from_slice(&f32_le(99.25)); // atp
    bytes.extend_from_slice(&50_000i32.to_le_bytes()); // volume
    bytes.extend_from_slice(&100i32.to_le_bytes()); // sell
    bytes.extend_from_slice(&200i32.to_le_bytes()); // buy
    bytes.extend_from_slice(&1_250_000i32.to_le_bytes()); // oi
    bytes.extend_from_slice(&1_300_000i32.to_le_bytes()); // oi day high
    bytes.extend_from_slice(&1_100_000i32.to_le_bytes()); // oi day low
    bytes.extend_from_slice(&f32_le(97.0)); // day open
    bytes.extend_from_slice(&f32_le(101.0)); // day close
    bytes.extend_from_slice(&f32_le(102.5)); // day high
    bytes.extend_from_slice(&f32_le(96.0)); // day low
    for level in 0..5u32 {
        let level_i = level as i32;
        let price = 100.0 - level as f32;
        bytes.extend_from_slice(&(1000 - level_i).to_le_bytes()); // bid qty
        bytes.extend_from_slice(&(2000 + level_i).to_le_bytes()); // ask qty
        bytes.extend_from_slice(&3i16.to_le_bytes()); // bid orders
        bytes.extend_from_slice(&4i16.to_le_bytes()); // ask orders
        bytes.extend_from_slice(&f32_le(price)); // bid price
        bytes.extend_from_slice(&f32_le(price + 1.0)); // ask price
    }

    let packet = tantra_market_dhan::protocol::decode_packet(&bytes).unwrap();
    match packet {
        FeedPacket::Full(_, data) => {
            assert_eq!(data.quote.last_price, 100.5);
            assert_eq!(data.open_interest, 1_250_000);
            assert_eq!(data.oi_day_high, 1_300_000);
            assert_eq!(data.oi_day_low, 1_100_000);
            assert_eq!(data.depth.len(), 5);
            assert_eq!(data.depth[0].bid_qty, 1000);
            assert_eq!(data.depth[0].ask_qty, 2000);
            assert_eq!(data.depth[0].bid_price, 100.0);
            assert_eq!(data.depth[0].ask_price, 101.0);
            assert_eq!(data.depth[4].bid_price, 96.0);
            assert_eq!(data.depth[4].ask_price, 97.0);
        }
        other => panic!("expected full packet, got {other:?}"),
    }
}

#[test]
fn disconnect_packet_and_unknown_code() {
    let mut bytes = header(response::DISCONNECT, 10, 1, 0);
    bytes.extend_from_slice(&805i16.to_le_bytes());
    match tantra_market_dhan::protocol::decode_packet(&bytes).unwrap() {
        FeedPacket::Disconnect { code } => assert_eq!(code, 805),
        other => panic!("expected disconnect packet, got {other:?}"),
    }

    let unknown = [99, 0, 0, 0, 0, 0, 0, 0];
    match tantra_market_dhan::protocol::decode_packet(&unknown).unwrap() {
        FeedPacket::Unknown { code } => assert_eq!(code, 99),
        other => panic!("expected unknown packet, got {other:?}"),
    }
}

#[test]
fn truncated_packets_are_rejected() {
    let err = tantra_market_dhan::protocol::decode_packet(&[2, 16, 0]).unwrap_err();
    assert!(matches!(err, ProtocolError::Truncated { .. }));

    let mut short_ticker = GOLDEN_HEADER.to_vec();
    short_ticker.extend_from_slice(&[0x00, 0x00, 0xC8]); // missing 4th ltp byte
    let err = tantra_market_dhan::protocol::decode_packet(&short_ticker).unwrap_err();
    assert!(matches!(err, ProtocolError::Truncated { .. }));
}

#[test]
fn exchange_segment_names_round_trip() {
    for segment in [
        ExchangeSegment::Index,
        ExchangeSegment::NseEq,
        ExchangeSegment::NseFno,
        ExchangeSegment::NseCurrency,
        ExchangeSegment::BseEq,
        ExchangeSegment::McxComm,
        ExchangeSegment::BseCurrency,
        ExchangeSegment::BseFno,
    ] {
        assert_eq!(ExchangeSegment::from_name(segment.name()), Some(segment));
        assert_eq!(ExchangeSegment::from_code(segment.code()), Some(segment));
    }
    assert_eq!(ExchangeSegment::from_code(6), None);
    assert_eq!(ExchangeSegment::from_name("NSE_INDEX"), None);
}

#[test]
fn subscribe_json_uses_dhan_pascal_case_and_chunk_limit() {
    let json = subscribe_json(
        request::SUBSCRIBE_QUOTE,
        &[DhanInstrument {
            exchange_segment: "NSE_EQ",
            security_id: "1333".to_string(),
        }],
    )
    .unwrap();
    assert_eq!(
        json,
        r#"{"RequestCode":17,"InstrumentCount":1,"InstrumentList":[{"ExchangeSegment":"NSE_EQ","SecurityId":"1333"}]}"#
    );

    assert!(subscribe_json(request::SUBSCRIBE_QUOTE, &[]).is_err());
    let too_many: Vec<DhanInstrument> = (0..101)
        .map(|i| DhanInstrument {
            exchange_segment: "NSE_EQ",
            security_id: i.to_string(),
        })
        .collect();
    let err = subscribe_json(request::SUBSCRIBE_QUOTE, &too_many).unwrap_err();
    assert!(matches!(err, ProtocolError::InvalidRequest(_)));
}

#[test]
fn disconnect_json_is_the_documented_message() {
    assert_eq!(disconnect_json(), r#"{"RequestCode":12}"#);
}

#[tokio::test]
async fn packets_convert_to_canonical_ticks_through_the_mapper() {
    let mapper = Arc::new(RwLock::new(SymbolMapper::new()));
    let instrument =
        InstrumentId::new(Exchange::Nfo, "NIFTY26NOV24000CE").with_token("49081".to_string());
    {
        let mut m = mapper.write().await;
        m.register("2|49081".to_string(), instrument.clone());
    }
    let now = Utc::now();

    // Ticker -> Ltp.
    let mut ticker = header(response::TICKER, 16, 2, 49081);
    ticker.extend_from_slice(&f32_le(100.5));
    ticker.extend_from_slice(&1_700_000_000i32.to_le_bytes());
    let tick = packet_to_tick(
        &tantra_market_dhan::protocol::decode_packet(&ticker).unwrap(),
        &mapper,
        now,
    )
    .await
    .expect("ticker maps to a tick");
    match tick {
        Tick::Ltp {
            instrument: got,
            price,
            volume,
            ..
        } => {
            assert_eq!(got, instrument);
            assert_eq!(price, Price::new(Decimal::from_f64_retain(100.5).unwrap()));
            assert_eq!(volume, Quantity::ZERO);
        }
        other => panic!("expected Ltp tick, got {other:?}"),
    }

    // Quote -> Ltp with volume.
    let mut quote = header(response::QUOTE, 50, 2, 49081);
    quote.extend_from_slice(&f32_le(100.5));
    quote.extend_from_slice(&25i16.to_le_bytes());
    quote.extend_from_slice(&1_700_000_000i32.to_le_bytes());
    quote.extend_from_slice(&f32_le(99.25));
    quote.extend_from_slice(&12_345i32.to_le_bytes());
    quote.extend_from_slice(&0i32.to_le_bytes());
    quote.extend_from_slice(&0i32.to_le_bytes());
    quote.extend_from_slice(&f32_le(97.0));
    quote.extend_from_slice(&f32_le(101.0));
    quote.extend_from_slice(&f32_le(102.5));
    quote.extend_from_slice(&f32_le(96.0));
    let tick = packet_to_tick(
        &tantra_market_dhan::protocol::decode_packet(&quote).unwrap(),
        &mapper,
        now,
    )
    .await
    .expect("quote maps to a tick");
    match tick {
        Tick::Ltp { volume, price, .. } => {
            assert_eq!(volume, Quantity::new(Decimal::from(12_345)));
            assert_eq!(price, Price::new(Decimal::from_f64_retain(100.5).unwrap()));
        }
        other => panic!("expected Ltp tick, got {other:?}"),
    }

    // An unmapped security id yields no tick (never a fabricated one).
    let mut unknown = header(response::TICKER, 16, 2, 999_999);
    unknown.extend_from_slice(&f32_le(100.5));
    unknown.extend_from_slice(&0i32.to_le_bytes());
    let decoded = tantra_market_dhan::protocol::decode_packet(&unknown).unwrap();
    assert!(
        packet_to_tick(&decoded, &mapper, now).await.is_none(),
        "unmapped wire instruments must be dropped, not fabricated"
    );
}

mod response {
    pub const TICKER: u8 = 2;
    pub const QUOTE: u8 = 4;
    pub const OPEN_INTEREST: u8 = 5;
    pub const PREV_CLOSE: u8 = 6;
    pub const FULL: u8 = 8;
    pub const DISCONNECT: u8 = 50;
}
