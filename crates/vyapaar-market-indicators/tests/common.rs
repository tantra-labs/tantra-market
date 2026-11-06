//! Shared fixtures for the extracted in-crate tests (ported from
//! `honba-me-indicators/src/tests/mod.rs`).
#![allow(dead_code)]

use chrono::Utc;
use rust_decimal::Decimal;
use vyapaar_core::{Bar, Exchange, InstrumentId, Price, Quantity};
use vyapaar_market_indicators::Indicator;

/// A placeholder instrument for tests where the instrument does not matter.
pub fn any_instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Nse, "X")
}

/// A bar with the given high, low and close (open = close, volume 1).
pub fn hlc(high: f64, low: f64, close: f64) -> Bar {
    Bar {
        instrument: any_instrument(),
        open: Price::new(Decimal::from_f64_retain(close).unwrap_or_default()),
        high: Price::new(Decimal::from_f64_retain(high).unwrap_or_default()),
        low: Price::new(Decimal::from_f64_retain(low).unwrap_or_default()),
        close: Price::new(Decimal::from_f64_retain(close).unwrap_or_default()),
        volume: Quantity::from_u64(1),
        timestamp: Utc::now(),
    }
}

/// A bar with the given high, low, close and volume.
pub fn hlcv(high: f64, low: f64, close: f64, volume: f64) -> Bar {
    Bar {
        instrument: any_instrument(),
        open: Price::new(Decimal::from_f64_retain(close).unwrap_or_default()),
        high: Price::new(Decimal::from_f64_retain(high).unwrap_or_default()),
        low: Price::new(Decimal::from_f64_retain(low).unwrap_or_default()),
        close: Price::new(Decimal::from_f64_retain(close).unwrap_or_default()),
        volume: Quantity::new(Decimal::from_f64_retain(volume).unwrap_or_default()),
        timestamp: Utc::now(),
    }
}

/// Feeds every input and returns the outputs in order.
pub fn feed<I>(ind: &mut I, inputs: &[f64]) -> Vec<Option<I::Output>>
where
    I: for<'a> Indicator<Input<'a> = f64>,
{
    inputs.iter().map(|&x| ind.update(x)).collect()
}

pub fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
