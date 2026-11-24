//! Unit tests for `crate::sma`.

mod common;
use common::feed;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Sma};

#[test]
#[should_panic(expected = "SMA period must be positive")]
fn zero_period_panics() {
    let _ = Sma::new(0);
}

#[test]
fn reports_its_period_and_readiness() {
    let mut s = Sma::new(2);
    assert_eq!(s.period(), 2);
    assert!(!s.is_ready());
    s.update(dec!(1));
    assert!(!s.is_ready());
    s.update(dec!(3));
    assert!(s.is_ready());
    assert_eq!(s.value(), Some(dec!(2)));
}

#[test]
fn period_one_echoes_the_input() {
    let mut s = Sma::new(1);
    assert_eq!(
        feed(&mut s, &[dec!(4), dec!(7), dec!(-1)]),
        [Some(dec!(4)), Some(dec!(7)), Some(dec!(-1))]
    );
}

#[test]
fn window_slides_over_the_last_period_inputs() {
    let mut s = Sma::new(3);
    assert_eq!(
        feed(&mut s, &[dec!(1), dec!(2), dec!(3), dec!(4), dec!(5)]),
        [None, None, Some(dec!(2)), Some(dec!(3)), Some(dec!(4))]
    );
}

#[test]
fn huge_period_does_not_preallocate() {
    // Used to abort the process: VecDeque::with_capacity(2**40).
    let mut s = Sma::new(1 << 40);
    assert_eq!(s.update(dec!(1)), None);
    assert_eq!(s.period(), 1 << 40);
}

#[test]
fn update_f64_shim_ignores_non_finite() {
    let mut s = Sma::new(2);
    assert_eq!(s.update_f64(1.0), None);
    assert_eq!(s.update_f64(f64::NAN), None);
    assert_eq!(s.update_f64(f64::INFINITY), None);
    assert_eq!(s.update_f64(3.0), Some(2.0));
}
