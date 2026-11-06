//! Unit tests for `crate::pivot_points`.

mod common;
use common::assert_close;
use vyapaar_market_indicators::{Indicator, PivotPoints, PivotStyle};

#[test]
fn classic_levels_match_floor_trader_formulas() {
    let mut p = PivotPoints::new(PivotStyle::Classic);
    let v = p.update_hlc(120.0, 90.0, 100.0).unwrap();
    assert_close(v.pivot, (120.0 + 90.0 + 100.0) / 3.0); // 103.33
    assert_close(v.r1, 2.0 * v.pivot - 90.0); // 116.67
    assert_close(v.s1, 2.0 * v.pivot - 120.0); // 86.67
    assert_close(v.r2, v.pivot + 30.0); // 133.33
    assert_close(v.s2, v.pivot - 30.0); // 73.33
    assert_close(v.r3.unwrap(), 120.0 + 2.0 * (v.pivot - 90.0)); // 146.67
    assert_close(v.s3.unwrap(), 90.0 - 2.0 * (120.0 - v.pivot)); // 56.67
}

#[test]
fn fibonacci_levels_are_retracements_of_the_range() {
    let mut p = PivotPoints::new(PivotStyle::Fibonacci);
    let v = p.update_hlc(120.0, 90.0, 100.0).unwrap();
    assert_close(v.r1, v.pivot + 0.382 * 30.0);
    assert_close(v.s1, v.pivot - 0.382 * 30.0);
    assert_close(v.r2, v.pivot + 0.618 * 30.0);
    assert_close(v.s2, v.pivot - 0.618 * 30.0);
    assert_close(v.r3.unwrap(), v.pivot + 30.0);
    assert_close(v.s3.unwrap(), v.pivot - 30.0);
}

#[test]
fn woodie_levels_weight_the_close_doubly() {
    let mut p = PivotPoints::new(PivotStyle::Woodie);
    let v = p.update_hlc(120.0, 90.0, 100.0).unwrap();
    assert_close(v.pivot, (120.0 + 90.0 + 2.0 * 100.0) / 4.0); // 102.5
    assert_close(v.r1, 2.0 * v.pivot - 90.0);
    assert_close(v.s1, 2.0 * v.pivot - 120.0);
    assert_eq!(v.r3, None);
    assert_eq!(v.s3, None);
}

#[test]
fn every_bar_yields_levels_from_the_first() {
    let mut p = PivotPoints::new(PivotStyle::Classic);
    assert!(p.update_hlc(10.0, 8.0, 9.0).is_some());
}

#[test]
fn reset_clears_the_last_value() {
    let mut p = PivotPoints::new(PivotStyle::Classic);
    p.update_hlc(10.0, 8.0, 9.0);
    p.reset();
    assert_eq!(p.value(), None);
    assert!(p.update_hlc(10.0, 8.0, 9.0).is_some());
}
