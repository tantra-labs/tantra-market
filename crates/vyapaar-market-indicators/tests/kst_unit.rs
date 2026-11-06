//! Unit tests for `crate::kst`.

mod common;
use common::assert_close;
use vyapaar_market_indicators::{Indicator, Kst};

#[test]
fn constant_prices_give_a_zero_kst() {
    let mut k = Kst::new();
    let mut last = None;
    for _ in 0..80 {
        last = k.update(100.0);
    }
    // Every ROC is zero, so every smoothed leg is zero.
    assert_close(last.unwrap(), 0.0);
}

#[test]
fn first_value_appears_after_forty_five_closes() {
    let mut k = Kst::new();
    for i in 0..44 {
        assert_eq!(
            k.update(100.0 + i as f64),
            None,
            "close {i} should be warmup"
        );
    }
    assert!(k.update(144.0).is_some());
}

#[test]
fn a_steady_trend_produces_a_stable_positive_kst() {
    let mut k = Kst::new();
    let mut last = None;
    for i in 1..=100 {
        last = k.update(i as f64);
    }
    // Linearly increasing closes → constant positive ROCs in every leg.
    let v = last.unwrap();
    assert!(v > 0.0, "expected a positive KST, got {v}");
    assert_close(v, k.value().unwrap());
}

#[test]
fn reset_requires_a_fresh_warmup() {
    let mut k = Kst::new();
    for i in 0..80 {
        k.update(100.0 + i as f64);
    }
    k.reset();
    assert_eq!(k.value(), None);
    assert_eq!(k.update(100.0), None);
}
