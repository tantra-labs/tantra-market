//! Unit tests for `crate::cmo`.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Cmo, Indicator};

fn feed(c: &mut Cmo, xs: &[Decimal]) -> Vec<Option<Decimal>> {
    xs.iter().map(|&x| c.update(x)).collect()
}

#[test]
#[should_panic(expected = "CMO period must be positive")]
fn zero_period_panics() {
    let _ = Cmo::new(0);
}

#[test]
fn all_up_moves_are_plus_one_hundred() {
    let mut c = Cmo::new(2);
    assert_eq!(
        feed(&mut c, &[dec!(1), dec!(2), dec!(3)]),
        [None, None, Some(dec!(100))]
    );
}

#[test]
fn mixed_moves_are_bounded_between_minus_and_plus_one_hundred() {
    let mut c = Cmo::new(2);
    let out = feed(&mut c, &[dec!(1), dec!(2), dec!(3), dec!(2), dec!(2.5)]);
    assert_eq!(out[0], None);
    assert_eq!(out[1], None);
    assert_eq!(out[2], Some(dec!(100))); // gains 1,1
    assert_eq!(out[3], Some(Decimal::ZERO)); // gains 1,1 losses 0,1
    // gains 0.5,1 losses 1,0 -> 100*(0.5 - 1)/(0.5 + 1) = -33.33...
    let expected = dec!(-100) / dec!(3);
    let diff = (out[4].unwrap() - expected).abs();
    assert!(diff < dec!(0.000000001), "{:?} != {expected}", out[4]);
}

#[test]
fn reset_clears_momentum_history() {
    let mut c = Cmo::new(2);
    feed(&mut c, &[dec!(1), dec!(2), dec!(3)]);
    c.reset();
    assert_eq!(c.value(), None);
    assert_eq!(
        feed(&mut c, &[dec!(10), dec!(10), dec!(10), dec!(20)]),
        [None, None, Some(Decimal::ZERO), Some(dec!(100))]
    );
}

#[test]
fn f64_shim_rejects_non_finite_without_mutation() {
    let mut c = Cmo::new(1);
    assert_eq!(c.update_f64(1.0), None);
    assert_eq!(c.update_f64(f64::NAN), None);
    assert_eq!(c.update_f64(2.0), Some(100.0));
}
