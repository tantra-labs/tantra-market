//! Unit tests for `crate::obv`.

mod common;
use common::hlcv;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Obv};

#[test]
fn tracks_prev_close_across_up_down_and_flat_bars() {
    let mut obv = Obv::new();
    // Bar 1 sets baseline close=100 -> OBV=0
    assert_eq!(obv.update(&hlcv(101.0, 99.0, 100.0, 500.0)), Some(dec!(0)));
    // Close rises 100 -> 102: add 200 -> OBV=200
    assert_eq!(
        obv.update(&hlcv(103.0, 100.0, 102.0, 200.0)),
        Some(dec!(200))
    );
    // Close drops 102 -> 101 (note: still > bar 1's 100!): subtract 150 -> OBV=50
    assert_eq!(
        obv.update(&hlcv(102.0, 100.0, 101.0, 150.0)),
        Some(dec!(50))
    );
    // Flat close 101 -> 101: unchanged -> OBV=50
    assert_eq!(
        obv.update(&hlcv(102.0, 100.0, 101.0, 999.0)),
        Some(dec!(50))
    );
    assert_eq!(obv.value(), Some(dec!(50)));
}

#[test]
fn reset_returns_to_initial_state() {
    let mut obv = Obv::new();
    obv.update(&hlcv(100.0, 99.0, 100.0, 10.0));
    obv.update(&hlcv(102.0, 100.0, 101.0, 25.0));
    obv.reset();
    assert_eq!(obv.update(&hlcv(102.0, 100.0, 100.0, 50.0)), Some(dec!(0)));
}
