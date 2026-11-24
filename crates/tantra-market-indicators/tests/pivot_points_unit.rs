//! Unit tests for `crate::pivot_points`.

mod common;
use common::assert_dec_close;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, PivotPoints, PivotStyle};

#[test]
fn classic_levels_match_floor_trader_formulas() {
    let mut p = PivotPoints::new(PivotStyle::Classic);
    let v = p.update_hlc(dec!(120), dec!(90), dec!(100)).unwrap();
    let expected_pivot = (dec!(120) + dec!(90) + dec!(100)) / dec!(3);
    assert_dec_close(v.pivot, expected_pivot);
    assert_dec_close(v.r1, dec!(2) * v.pivot - dec!(90));
    assert_dec_close(v.s1, dec!(2) * v.pivot - dec!(120));
    assert_dec_close(v.r2, v.pivot + dec!(30));
    assert_dec_close(v.s2, v.pivot - dec!(30));
    assert_dec_close(v.r3.unwrap(), dec!(120) + dec!(2) * (v.pivot - dec!(90)));
    assert_dec_close(v.s3.unwrap(), dec!(90) - dec!(2) * (dec!(120) - v.pivot));
}

#[test]
fn fibonacci_levels_are_retracements_of_the_range() {
    let mut p = PivotPoints::new(PivotStyle::Fibonacci);
    let v = p.update_hlc(dec!(120), dec!(90), dec!(100)).unwrap();
    assert_dec_close(v.r1, v.pivot + dec!(0.382) * dec!(30));
    assert_dec_close(v.s1, v.pivot - dec!(0.382) * dec!(30));
    assert_dec_close(v.r2, v.pivot + dec!(0.618) * dec!(30));
    assert_dec_close(v.s2, v.pivot - dec!(0.618) * dec!(30));
    assert_dec_close(v.r3.unwrap(), v.pivot + dec!(30));
    assert_dec_close(v.s3.unwrap(), v.pivot - dec!(30));
}

#[test]
fn woodie_levels_weight_the_close_doubly() {
    let mut p = PivotPoints::new(PivotStyle::Woodie);
    let v = p.update_hlc(dec!(120), dec!(90), dec!(100)).unwrap();
    assert_dec_close(v.pivot, (dec!(120) + dec!(90) + dec!(200)) / dec!(4));
    assert_dec_close(v.r1, dec!(2) * v.pivot - dec!(90));
    assert_dec_close(v.s1, dec!(2) * v.pivot - dec!(120));
    assert_eq!(v.r3, None);
    assert_eq!(v.s3, None);
}

#[test]
fn every_bar_yields_levels_from_the_first() {
    let mut p = PivotPoints::new(PivotStyle::Classic);
    assert!(p.update_hlc(dec!(10), dec!(8), dec!(9)).is_some());
}

#[test]
fn reset_clears_the_last_value() {
    let mut p = PivotPoints::new(PivotStyle::Classic);
    p.update_hlc(dec!(10), dec!(8), dec!(9));
    p.reset();
    assert_eq!(p.value(), None);
    assert!(p.update_hlc(dec!(10), dec!(8), dec!(9)).is_some());
}
