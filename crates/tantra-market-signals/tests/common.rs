//! Shared fixtures for the extracted signal-engine tests (ported from
//! `honba-me-signals/src/tests/mod.rs`).
#![allow(dead_code)]

use chrono::{Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};

/// A placeholder instrument for tests where the instrument does not matter.
pub fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Nse, "X")
}

/// Helper to convert f64 to Price
pub fn price(f: f64) -> Price {
    Price::new(Decimal::from_f64(f).unwrap())
}

/// Helper to convert f64 to Quantity
pub fn quantity(f: f64) -> Quantity {
    Quantity::new(Decimal::from_f64(f).unwrap())
}

/// Bars with `open == high == low == close`, at one-minute steps.
pub fn bars(closes: &[f64], volumes: &[f64]) -> Vec<Bar> {
    let id = instrument();
    let start_ts = Utc.with_ymd_and_hms(2025, 1, 1, 9, 15, 0).unwrap();
    closes
        .iter()
        .zip(volumes)
        .enumerate()
        .map(|(i, (&c, &v))| {
            let ts = start_ts + Duration::minutes(i as i64);
            Bar {
                instrument: id.clone(),
                open: price(c),
                high: price(c),
                low: price(c),
                close: price(c),
                volume: quantity(v),
                timestamp: ts,
            }
        })
        .collect()
}

/// Bars with explicit OHLC and volume, at one-minute steps.
pub fn bars_ohlcv(
    open: &[f64],
    high: &[f64],
    low: &[f64],
    close: &[f64],
    volume: &[f64],
) -> Vec<Bar> {
    let id = instrument();
    let start_ts = Utc.with_ymd_and_hms(2025, 1, 1, 9, 15, 0).unwrap();
    (0..close.len())
        .map(|i| {
            let ts = start_ts + Duration::minutes(i as i64);
            Bar {
                instrument: id.clone(),
                open: price(open[i]),
                high: price(high[i]),
                low: price(low[i]),
                close: price(close[i]),
                volume: quantity(volume[i]),
                timestamp: ts,
            }
        })
        .collect()
}

/// Bars with explicit OHLC, volume and a UTC day number per bar: timestamp is the day boundary
/// plus `i` minutes, so bars on the same day share a day but stay ordered.
pub fn bars_at_days(
    open: &[f64],
    high: &[f64],
    low: &[f64],
    close: &[f64],
    volume: &[f64],
    days: &[u64],
) -> Vec<Bar> {
    let id = instrument();
    let start_ts = Utc.with_ymd_and_hms(2025, 1, 1, 9, 15, 0).unwrap();
    (0..close.len())
        .map(|i| {
            let ts = start_ts + Duration::days(days[i] as i64) + Duration::minutes(i as i64);
            Bar {
                instrument: id.clone(),
                open: price(open[i]),
                high: price(high[i]),
                low: price(low[i]),
                close: price(close[i]),
                volume: quantity(volume[i]),
                timestamp: ts,
            }
        })
        .collect()
}

/// A long, deterministic oscillation with occasional volume spikes: enough variety for every
/// detector to fire several times over the window.
pub fn wavy() -> Vec<Bar> {
    let mut closes = Vec::new();
    let mut volumes = Vec::new();
    for i in 0..200 {
        let t = i as f64;
        closes.push(100.0 + 15.0 * (t / 6.0).sin() + 5.0 * (t / 2.5).cos());
        volumes.push(if i % 17 == 0 { 5_000.0 } else { 1_000.0 });
    }
    bars(&closes, &volumes)
}
