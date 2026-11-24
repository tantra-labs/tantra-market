//! Unit tests for `crate::vwap`.

mod common;
use common::hlcv;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Vwap};

#[test]
fn computes_exact_volume_weighted_average_price() {
    let mut vwap = Vwap::new();
    // Bar 1: TP = (12 + 8 + 10)/3 = 10, vol = 100 -> VWAP = 10
    assert_eq!(vwap.update(&hlcv(12.0, 8.0, 10.0, 100.0)), Some(dec!(10)));
    // Bar 2: TP = (22 + 18 + 20)/3 = 20, vol = 300 -> VWAP = (1000 + 6000)/400 = 17.5
    assert_eq!(
        vwap.update(&hlcv(22.0, 18.0, 20.0, 300.0)),
        Some(dec!(17.5))
    );
    assert_eq!(vwap.value(), Some(dec!(17.5)));
}

#[test]
fn zero_volume_falls_back_to_typical_price() {
    let mut vwap = Vwap::new();
    assert_eq!(vwap.update(&hlcv(12.0, 8.0, 10.0, 0.0)), Some(dec!(10)));
}

#[test]
fn reset_clears_cumulative_sums() {
    let mut vwap = Vwap::new();
    vwap.update(&hlcv(12.0, 8.0, 10.0, 100.0));
    vwap.reset();
    assert_eq!(vwap.value(), None);
    assert_eq!(vwap.update(&hlcv(22.0, 18.0, 20.0, 50.0)), Some(dec!(20)));
}
