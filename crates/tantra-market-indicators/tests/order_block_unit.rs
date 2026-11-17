//! Unit tests for [`tantra_market_indicators::OrderBlock`].

use std::str::FromStr;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use tantra_core::levels::LevelSide;
use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
use tantra_market_indicators::{Indicator, OrderBlock, OrderBlockValue};

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// A bar at an explicit timestamp (04:00 UTC, minute 0).
fn bar(open: i32, high: i32, low: i32, close: i32) -> Bar {
    Bar {
        instrument: InstrumentId::new(Exchange::Nse, "X"),
        open: Price::new(Decimal::from(open)),
        high: Price::new(Decimal::from(high)),
        low: Price::new(Decimal::from(low)),
        close: Price::new(Decimal::from(close)),
        volume: Quantity::from_u64(1),
        timestamp: Utc.with_ymd_and_hms(2026, 1, 5, 4, 0, 0).single().unwrap(),
    }
}

/// A displacement detector with ATR period 3, multiplier 1.5 and body ratio
/// 0.5.
fn detector() -> OrderBlock {
    OrderBlock::new(dec("1.5"), dec("0.5"), 3)
}

/// Feeds the three seed bars (each TR = 3, so the seeded ATR is exactly 3).
fn seed(det: &mut OrderBlock) {
    assert_eq!(det.update(&bar(100, 103, 100, 100)), None); // Flat
    assert_eq!(det.update(&bar(100, 103, 100, 100)), None); // Flat
    assert_eq!(det.update(&bar(99, 102, 99, 101)), None); // Up, body [99, 101]
}

/// ATR warm-up: nothing is emitted until `atr_period` bars have accumulated,
/// and the seed bars themselves are not displacement candles (range 3 < 4.5).
#[test]
fn atr_warm_up_returns_none() {
    let mut ob = detector();
    assert_eq!(ob.update(&bar(100, 103, 100, 100)), None);
    assert_eq!(ob.update(&bar(100, 103, 100, 100)), None);
    // Third bar primes ATR = (3 + 3 + 3) / 3 = 3, but it is not a displacement.
    assert_eq!(ob.update(&bar(99, 102, 99, 101)), None);
    assert_eq!(ob.value(), None);
}

/// Bearish displacement: the preceding bullish candle's body range becomes a
/// `Resistance` zone, emitted once on the displacement bar.
///
/// The displacement bar `(101, 101, 95, 95)` has TR 6 (gapping below the seed's
/// close of 101), so the updated ATR is `(3*2 + 6)/3 = 4` and the threshold is
/// `1.5 * 4 = 6`; its range is exactly 6 and its body ratio is 1.
#[test]
fn bearish_displacement_emits_resistance_zone() {
    let mut ob = detector();
    seed(&mut ob);

    let v = ob.update(&bar(101, 101, 95, 95)).unwrap();
    let expected = OrderBlockValue {
        side: LevelSide::Resistance,
        low: Decimal::from(99),
        high: Decimal::from(101),
    };
    assert_eq!(v, expected);
    assert_eq!(ob.value(), Some(expected));

    // A later, non-displacement bar emits nothing and clears the last value.
    assert_eq!(ob.update(&bar(100, 102, 98, 101)), None);
    assert_eq!(ob.value(), None);
}

/// Bullish displacement maps to a `Support` zone.
///
/// The seed's last bar is a Down candle with body [100, 101]; the displacement
/// bar `(100, 106, 100, 106)` opens at the prior close, so TR == range == 6 and
/// the updated ATR is 4 -> threshold 6.
#[test]
fn bullish_displacement_emits_support_zone() {
    let mut ob = detector();
    assert_eq!(ob.update(&bar(100, 103, 100, 100)), None);
    assert_eq!(ob.update(&bar(100, 103, 100, 100)), None);
    assert_eq!(ob.update(&bar(101, 102, 99, 100)), None); // Down, body [100, 101]

    let v = ob.update(&bar(100, 106, 100, 106)).unwrap();
    let expected = OrderBlockValue {
        side: LevelSide::Support,
        low: Decimal::from(100),
        high: Decimal::from(101),
    };
    assert_eq!(v, expected);
}

/// A large range alone is not a displacement: the body-ratio filter rejects
/// wide candles with small bodies, pure dojis, and zero-range bars.
#[test]
fn body_ratio_filter_rejects_thin_bodies() {
    let mut ob = detector();
    seed(&mut ob);

    // Range 7 passes the ATR-multiplier gate (threshold 6.5) but the body
    // ratio 1/7 is below 0.5 -> not a displacement.
    assert_eq!(ob.update(&bar(100, 103, 96, 99)), None);
    // A pure doji (open == close) has body ratio 0, whatever the range.
    assert_eq!(ob.update(&bar(100, 120, 80, 100)), None);
    // A zero-range bar cannot be a displacement at all.
    assert_eq!(ob.update(&bar(100, 100, 100, 100)), None);
    assert_eq!(ob.value(), None);
}

/// The scan walks back through same-direction candles to the most recent
/// opposing candle.
#[test]
fn scans_back_to_last_opposing_candle() {
    let mut ob = detector();
    seed(&mut ob);
    assert_eq!(ob.update(&bar(99, 102, 99, 101)), None); // Up, body [99, 101]
    assert_eq!(ob.update(&bar(101, 103, 100, 102)), None); // Up, body [100, 102]

    // Displacement Down: the scan skips the two recent Up candles and lands on
    // the most recent opposing (Up) bar's body range [101, 102] - not the
    // older seed bar's [99, 101]. The displacement bar opens at the prior
    // close (102) so TR == range == 6 and the threshold is 1.5 * 4 = 6.
    let v = ob.update(&bar(102, 102, 96, 96)).unwrap();
    assert_eq!(v.side, LevelSide::Resistance);
    assert_eq!(v.low, Decimal::from(101));
    assert_eq!(v.high, Decimal::from(102));
}

/// `reset` clears the ATR, the bar history and the last value.
#[test]
fn reset_clears_state() {
    let mut ob = detector();
    seed(&mut ob);
    assert!(ob.update(&bar(101, 101, 95, 95)).is_some());
    ob.reset();
    assert_eq!(ob.value(), None);
    // ATR must warm up again from scratch.
    assert_eq!(ob.update(&bar(100, 103, 100, 100)), None);
    assert_eq!(ob.update(&bar(100, 103, 100, 100)), None);
    assert_eq!(ob.update(&bar(99, 102, 99, 101)), None);
}
