//! Shared fixtures for `tantra-market-indicators` unit tests.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
use tantra_market_indicators::Indicator;

/// A deterministic timestamp for unit tests (Zero-Clock purity).
pub fn fixed_ts() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
}

/// A placeholder instrument for tests where the instrument does not matter.
pub fn any_instrument() -> InstrumentId {
    InstrumentId::try_new(Exchange::Nse, "X").unwrap()
}

/// A `Decimal` helper from integers/fractional `f64` literal.
pub fn dec_f64(v: f64) -> Decimal {
    Decimal::try_from(v).expect("finite test literal")
}

/// A bar with the given `Decimal` high, low and close (open = close, volume 1).
pub fn hlc_dec(high: Decimal, low: Decimal, close: Decimal) -> Bar {
    Bar {
        instrument: any_instrument(),
        open: Price::try_new(close).unwrap(),
        high: Price::try_new(high).unwrap(),
        low: Price::try_new(low).unwrap(),
        close: Price::try_new(close).unwrap(),
        volume: Quantity::from_u64(1),
        timestamp: fixed_ts(),
    }
}

/// A bar with the given `Decimal` high, low, close and volume.
pub fn hlcv_dec(high: Decimal, low: Decimal, close: Decimal, volume: Decimal) -> Bar {
    Bar {
        instrument: any_instrument(),
        open: Price::try_new(close).unwrap(),
        high: Price::try_new(high).unwrap(),
        low: Price::try_new(low).unwrap(),
        close: Price::try_new(close).unwrap(),
        volume: Quantity::try_new(volume).unwrap(),
        timestamp: fixed_ts(),
    }
}

/// Convenience `f64`-literal wrapper constructing an exact `Decimal` bar.
pub fn hlc(high: f64, low: f64, close: f64) -> Bar {
    hlc_dec(dec_f64(high), dec_f64(low), dec_f64(close))
}

/// Convenience `f64`-literal wrapper constructing an exact `Decimal` bar with volume.
pub fn hlcv(high: f64, low: f64, close: f64, volume: f64) -> Bar {
    hlcv_dec(dec_f64(high), dec_f64(low), dec_f64(close), dec_f64(volume))
}

/// Feeds every `Decimal` input and returns the outputs in order.
pub fn feed<I>(ind: &mut I, inputs: &[Decimal]) -> Vec<Option<I::Output>>
where
    I: for<'a> Indicator<Input<'a> = Decimal>,
{
    inputs.iter().map(|&x| ind.update(x)).collect()
}

/// Asserts two `Decimal` values are within `tol` (`1e-9` by default).
pub fn assert_dec_close(a: Decimal, b: Decimal) {
    let tol = Decimal::new(1, 9);
    assert!((a - b).abs() < tol, "{a} != {b} (diff {})", (a - b).abs());
}

/// Asserts two `Decimal` values are within a wider tolerance `tol`.
pub fn assert_dec_within(a: Decimal, b: Decimal, tol: Decimal) {
    assert!((a - b).abs() < tol, "{a} != {b} (diff {})", (a - b).abs());
}
