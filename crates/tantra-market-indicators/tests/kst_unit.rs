//! Unit tests for `crate::kst`.

mod common;
use common::assert_dec_close;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Kst};

#[test]
fn constant_prices_give_a_zero_kst() {
    let mut k = Kst::new();
    let mut last = None;
    for _ in 0..80 {
        last = k.update(dec!(100));
    }
    // Every ROC is zero, so every smoothed leg is zero.
    assert_dec_close(last.unwrap(), dec!(0));
}

#[test]
fn first_value_appears_after_forty_five_closes() {
    let mut k = Kst::new();
    for i in 0..44 {
        assert_eq!(
            k.update(Decimal::from(100 + i)),
            None,
            "close {i} should be warmup"
        );
    }
    assert!(k.update(dec!(144)).is_some());
}

#[test]
fn a_steady_trend_produces_a_stable_positive_kst() {
    let mut k = Kst::new();
    let mut last = None;
    for i in 1..=100 {
        last = k.update(Decimal::from(i));
    }
    // Linearly increasing closes -> constant positive ROCs in every leg.
    let v = last.unwrap();
    assert!(v > Decimal::ZERO, "expected a positive KST, got {v}");
    assert_dec_close(v, k.value().unwrap());
}

#[test]
fn reset_requires_a_fresh_warmup() {
    let mut k = Kst::new();
    for i in 0..80 {
        k.update(Decimal::from(100 + i));
    }
    k.reset();
    assert_eq!(k.value(), None);
    assert_eq!(k.update(dec!(100)), None);
}
