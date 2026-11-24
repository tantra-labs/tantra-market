//! Unit tests for `crate::fisher_transform`.

mod common;
use common::assert_dec_close;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{FisherTransform, Indicator};

#[test]
#[should_panic(expected = "Fisher period must be positive")]
fn zero_period_panics() {
    let _ = FisherTransform::new(0);
}

#[test]
fn first_value_appears_after_period_inputs() {
    let mut f = FisherTransform::new(3);
    assert_eq!(f.period(), 3);
    assert_eq!(f.update(dec!(1)), None);
    assert_eq!(f.update(dec!(2)), None);
    let v = f.update(dec!(3)).unwrap();
    // scaled = 2*(3-1)/(3-1) - 1 = 1; raw = 0.66*1 = 0.66;
    // fisher = 0.5 * ln(1.66/0.34) ≈ 0.7928136319
    assert_dec_close(v, dec!(0.7928136319));
}

#[test]
fn rising_prices_push_fisher_positive() {
    let mut f = FisherTransform::new(3);
    f.update(dec!(1));
    f.update(dec!(2));
    let v3 = f.update(dec!(3)).unwrap();
    let v4 = f.update(dec!(4)).unwrap();
    assert!(v4 > v3);
    assert!(v4 > Decimal::ZERO);
}

#[test]
fn constant_price_converges_to_zero() {
    let mut f = FisherTransform::new(3);
    let mut last = None;
    for _ in 0..100 {
        last = f.update(dec!(10));
    }
    // Range is zero after warmup, so raw stays at its initial 0 -> fisher 0.
    assert_dec_close(last.unwrap(), dec!(0));
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut f = FisherTransform::new(3);
    f.update(dec!(1));
    f.update(dec!(2));
    f.update(dec!(3));
    f.reset();
    assert_eq!(f.value(), None);
    assert_eq!(f.update(dec!(1)), None);
    assert_eq!(f.update(dec!(2)), None);
    assert!(f.update(dec!(3)).is_some());
}
