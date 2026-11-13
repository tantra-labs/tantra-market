//! Golden-byte tests for the Zerodha Kite Connect 3 ticker wire protocol (`docs/gaps/0003`).
//!
//! Byte offsets are transcribed from the official Kite Connect 3 WebSocket docs. Packets
//! carry distinct values per field so a misplaced offset fails the assertion.

use std::sync::Arc;

use chrono::Utc;
use rust_decimal::Decimal;
use tantra_core::{Exchange, InstrumentId, Price, Tick};
use tantra_market_core::{Mode, SymbolMapper};
use tantra_market_zerodha::packet_to_tick;
use tantra_market_zerodha::protocol::{
    FeedPacket, Packet, ProtocolError, action, decode_message, decode_packet, mode_json,
    subscribe_json,
};
use tokio::sync::RwLock;

fn i32_le(value: i32) -> [u8; 4] {
    value.to_le_bytes()
}

/// Builds a single-packet message frame from a packet payload.
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![1, 0];
    bytes.extend_from_slice(&(payload.len() as i16).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn ltp_packet(token: u32, last_price: i32) -> Vec<u8> {
    let mut bytes = token.to_le_bytes().to_vec();
    bytes.extend_from_slice(&i32_le(last_price));
    bytes
}

#[test]
fn frame_with_single_ltp_packet_decodes() {
    let payload = ltp_packet(408_065, 141_295); // INFY at Rs 1412.95
    let packets = decode_message(&frame(&payload)).unwrap();
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].token, 408_065);
    assert_eq!(packets[0].len, 8);

    let decoded = decode_packet(Mode::Ltp, &packets[0]).unwrap();
    match decoded {
        FeedPacket::Ltp { token, last_price } => {
            assert_eq!(token, 408_065);
            assert_eq!(last_price, 141_295);
        }
        other => panic!("expected ltp packet, got {other:?}"),
    }
}

#[test]
fn frame_with_two_packets_parses_count_and_lengths() {
    let a = ltp_packet(408_065, 141_295);
    let b = ltp_packet(884_737, 4_520_000); // TATAMOTORS at Rs 45200
    let mut bytes = vec![2, 0];
    bytes.extend_from_slice(&(a.len() as i16).to_le_bytes());
    bytes.extend_from_slice(&a);
    bytes.extend_from_slice(&(b.len() as i16).to_le_bytes());
    bytes.extend_from_slice(&b);

    let packets = decode_message(&bytes).unwrap();
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0].token, 408_065);
    assert_eq!(packets[1].token, 884_737);
}

#[test]
fn instrument_quote_packet_decodes_every_field() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&408_065u32.to_le_bytes()); // token
    payload.extend_from_slice(&i32_le(141_295)); // ltp
    payload.extend_from_slice(&i32_le(5)); // ltq
    payload.extend_from_slice(&i32_le(141_247)); // atp
    payload.extend_from_slice(&i32_le(7_360_198)); // volume
    payload.extend_from_slice(&i32_le(0)); // total buy
    payload.extend_from_slice(&i32_le(5191)); // total sell
    payload.extend_from_slice(&i32_le(139_600)); // open
    payload.extend_from_slice(&i32_le(142_175)); // high
    payload.extend_from_slice(&i32_le(139_555)); // low
    payload.extend_from_slice(&i32_le(138_965)); // close
    assert_eq!(payload.len(), 44);

    let decoded = decode_packet(
        Mode::Quote,
        &Packet {
            len: 44,
            token: 408_065,
            payload: payload.clone(),
        },
    )
    .unwrap();
    match decoded {
        FeedPacket::Instrument(quote) => {
            assert_eq!(quote.token, 408_065);
            assert_eq!(quote.last_price, 141_295);
            assert_eq!(quote.last_traded_qty, 5);
            assert_eq!(quote.average_traded_price, 141_247);
            assert_eq!(quote.volume, 7_360_198);
            assert_eq!(quote.total_sell_qty, 5191);
            assert_eq!(quote.open, 139_600);
            assert_eq!(quote.high, 142_175);
            assert_eq!(quote.low, 139_555);
            assert_eq!(quote.close, 138_965);
            assert!(quote.depth.is_none(), "quote mode has no depth");
        }
        other => panic!("expected instrument quote, got {other:?}"),
    }
}

#[test]
fn instrument_full_packet_decodes_oi_and_ten_depth_entries() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&408_065u32.to_le_bytes()); // token
    for _ in 0..10 {
        payload.extend_from_slice(&i32_le(0));
    } // ltp..close (zeros)
    payload.extend_from_slice(&i32_le(1_700_000_000)); // ltt
    payload.extend_from_slice(&i32_le(1_250_000)); // oi
    payload.extend_from_slice(&i32_le(1_300_000)); // oi high
    payload.extend_from_slice(&i32_le(1_100_000)); // oi low
    payload.extend_from_slice(&i32_le(1_700_000_001)); // exchange ts
    for level in 0..10u32 {
        let level_i = level as i32;
        payload.extend_from_slice(&i32_le((level_i + 1) * 100)); // quantity
        payload.extend_from_slice(&i32_le((level_i + 1) * 50)); // price
        payload.extend_from_slice(&((level_i + 1) as i16).to_le_bytes()); // orders
        payload.extend_from_slice(&[0, 0]); // padding
    }
    assert_eq!(payload.len(), 184);

    let decoded = decode_packet(
        Mode::Full,
        &Packet {
            len: 184,
            token: 408_065,
            payload,
        },
    )
    .unwrap();
    match decoded {
        FeedPacket::Instrument(quote) => {
            assert_eq!(quote.last_traded_timestamp, Some(1_700_000_000));
            assert_eq!(quote.open_interest, Some(1_250_000));
            assert_eq!(quote.oi_day_high, Some(1_300_000));
            assert_eq!(quote.oi_day_low, Some(1_100_000));
            assert_eq!(quote.exchange_timestamp, Some(1_700_000_001));
            let depth = quote.depth.unwrap();
            assert_eq!(depth[0].quantity, 100);
            assert_eq!(depth[0].price, 50);
            assert_eq!(depth[0].orders, 1);
            assert_eq!(depth[9].quantity, 1000);
            assert_eq!(depth[9].price, 500);
            assert_eq!(depth[9].orders, 10);
        }
        other => panic!("expected instrument full packet, got {other:?}"),
    }
}

#[test]
fn index_quote_and_full_packets_decode() {
    // Index quote: token, ltp, high, low, open, close, price change (28 bytes).
    let mut quote = Vec::new();
    quote.extend_from_slice(&2_064_234u32.to_le_bytes()); // token (NIFTY index)
    quote.extend_from_slice(&i32_le(1_967_250)); // ltp
    quote.extend_from_slice(&i32_le(1_975_000)); // high
    quote.extend_from_slice(&i32_le(1_960_000)); // low
    quote.extend_from_slice(&i32_le(1_965_000)); // open
    quote.extend_from_slice(&i32_le(1_950_000)); // close
    quote.extend_from_slice(&i32_le(17_250)); // price change
    assert_eq!(quote.len(), 28);
    match decode_packet(
        Mode::Quote,
        &Packet {
            len: 28,
            token: 2_064_234,
            payload: quote,
        },
    )
    .unwrap()
    {
        FeedPacket::Index(index) => {
            assert_eq!(index.last_price, 1_967_250);
            assert_eq!(index.high, 1_975_000);
            assert_eq!(index.price_change, Some(17_250));
            assert!(index.exchange_timestamp.is_none());
        }
        other => panic!("expected index quote, got {other:?}"),
    }

    // Index full: adds change + exchange timestamp (32 bytes).
    let mut full = Vec::new();
    full.extend_from_slice(&2_064_234u32.to_le_bytes());
    full.extend_from_slice(&i32_le(1_967_250));
    full.extend_from_slice(&i32_le(1_975_000));
    full.extend_from_slice(&i32_le(1_960_000));
    full.extend_from_slice(&i32_le(1_965_000));
    full.extend_from_slice(&i32_le(1_950_000));
    full.extend_from_slice(&i32_le(1_7250)); // change
    full.extend_from_slice(&i32_le(1_700_000_000)); // exchange ts
    assert_eq!(full.len(), 32);
    match decode_packet(
        Mode::Full,
        &Packet {
            len: 32,
            token: 2_064_234,
            payload: full,
        },
    )
    .unwrap()
    {
        FeedPacket::Index(index) => {
            assert_eq!(index.price_change, Some(17_250));
            assert_eq!(index.exchange_timestamp, Some(1_700_000_000));
        }
        other => panic!("expected index full, got {other:?}"),
    }
}

#[test]
fn malformed_frames_are_rejected() {
    // Shorter than the 2-byte count.
    assert!(matches!(
        decode_message(&[1]).unwrap_err(),
        ProtocolError::Malformed(_)
    ));
    // Packet length overruns the frame.
    let bytes = vec![1, 0, 60, 0, 0, 0];
    assert!(matches!(
        decode_message(&bytes).unwrap_err(),
        ProtocolError::Malformed(_)
    ));
    // Packet count beyond the cap.
    let big = vec![0xB9, 0x0B]; // 3001 packets
    assert!(matches!(
        decode_message(&big).unwrap_err(),
        ProtocolError::Malformed(_)
    ));
    // A quote packet of an unexpected length for its mode.
    let packet = Packet {
        len: 20,
        token: 1,
        payload: vec![0; 20],
    };
    assert!(matches!(
        decode_packet(Mode::Quote, &packet).unwrap_err(),
        ProtocolError::UnexpectedLength { .. }
    ));
}

#[test]
fn request_json_shapes_match_kite() {
    assert_eq!(
        subscribe_json(action::SUBSCRIBE, &[408_065, 884_737]),
        r#"{"a":"subscribe","v":[408065,884737]}"#
    );
    assert_eq!(
        mode_json("full", &[408_065]),
        r#"{"a":"mode","v":["full",[408065]]}"#
    );
}

#[tokio::test]
async fn packets_convert_to_canonical_ticks() {
    let modes = Arc::new(RwLock::new(std::collections::HashMap::new()));
    let mapper = Arc::new(RwLock::new(SymbolMapper::new()));
    let instrument = InstrumentId::new(Exchange::Nse, "INFY").with_token("408065".to_string());
    {
        let mut m = mapper.write().await;
        m.register("408065".to_string(), instrument.clone());
        let mut modes = modes.write().await;
        modes.insert(408_065, Mode::Ltp);
    }
    let now = Utc::now();

    let payload = ltp_packet(408_065, 141_295);
    let packet = decode_message(&frame(&payload)).unwrap().remove(0);
    let tick = packet_to_tick(&packet, &modes, &mapper, now).await.unwrap();
    match tick {
        Tick::Ltp {
            instrument: got,
            price,
            ..
        } => {
            assert_eq!(got, instrument);
            assert_eq!(
                price,
                Price::new(Decimal::from(141_295) / Decimal::from(100))
            );
        }
        other => panic!("expected Ltp tick, got {other:?}"),
    }

    // Currency instruments divide by 10^7 instead of 100.
    let usdinr = InstrumentId::new(Exchange::Cds, "USDINR26NOVFUT").with_token("7".to_string());
    {
        let mut m = mapper.write().await;
        m.register("7".to_string(), usdinr.clone());
        let mut modes = modes.write().await;
        modes.insert(7, Mode::Ltp);
    }
    let payload = ltp_packet(7, 8_429_105); // 84.29105
    let packet = decode_message(&frame(&payload)).unwrap().remove(0);
    let tick = packet_to_tick(&packet, &modes, &mapper, now).await.unwrap();
    match tick {
        Tick::Ltp {
            instrument: got,
            price,
            ..
        } => {
            assert_eq!(got, usdinr);
            assert_eq!(
                price.value(),
                Decimal::from(8_429_105) / Decimal::from(10_000_000)
            );
        }
        other => panic!("expected Ltp tick, got {other:?}"),
    }

    // An unregistered token yields no tick.
    let payload = ltp_packet(999_999, 100);
    let packet = decode_message(&frame(&payload)).unwrap().remove(0);
    assert!(
        packet_to_tick(&packet, &modes, &mapper, now)
            .await
            .is_none(),
        "unregistered tokens must be dropped, not fabricated"
    );
}
