//! Unit tests for `crate::fisher_transform`.

mod common;
use common::assert_close;
use vyapaar_market_indicators::{FisherTransform, Indicator};

#[test]
#[should_panic(expected = "Fisher period must be positive")]
fn zero_period_panics() {
    let _ = FisherTransform::new(0);
}

#[test]
fn first_value_appears_after_period_inputs() {
    let mut f = FisherTransform::new(3);
    assert_eq!(f.period(), 3);
    assert_eq!(f.update(1.0), None);
    assert_eq!(f.update(2.0), None);
    let v = f.update(3.0).unwrap();
    // scaled = 2*(3-1)/(3-1) - 1 = 1; raw = 0.66*1 = 0.66;
    // fisher = 0.5 * ln(1.66/0.34) ≈ 0.7928136319
    assert_close(v, 0.792_813_631_9);
}

#[test]
fn rising_prices_push_fisher_positive() {
    let mut f = FisherTransform::new(3);
    f.update(1.0);
    f.update(2.0);
    let v3 = f.update(3.0).unwrap();
    let v4 = f.update(4.0).unwrap();
    assert!(v4 > v3);
    assert!(v4 > 0.0);
}

#[test]
fn constant_price_converges_to_zero() {
    let mut f = FisherTransform::new(3);
    let mut last = None;
    for _ in 0..100 {
        last = f.update(10.0);
    }
    // Range is zero after warmup, so raw stays at its initial 0 → fisher 0.
    assert_close(last.unwrap(), 0.0);
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut f = FisherTransform::new(3);
    f.update(1.0);
    f.update(2.0);
    f.update(3.0);
    f.reset();
    assert_eq!(f.value(), None);
    assert_eq!(f.update(1.0), None);
    assert_eq!(f.update(2.0), None);
    assert!(f.update(3.0).is_some());
}
