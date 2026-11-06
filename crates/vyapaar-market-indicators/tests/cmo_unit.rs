//! Unit tests for `crate::cmo`.

mod common;
use common::feed;
use vyapaar_market_indicators::{Cmo, Indicator};

#[test]
#[should_panic(expected = "CMO period must be positive")]
fn zero_period_panics() {
    let _ = Cmo::new(0);
}

#[test]
fn all_up_moves_are_plus_one_hundred() {
    let mut c = Cmo::new(2);
    assert_eq!(feed(&mut c, &[1.0, 2.0, 3.0]), [None, None, Some(100.0)]);
}

#[test]
fn mixed_moves_are_bounded_between_minus_and_plus_one_hundred() {
    // Two up, then two down, then mixed: verify the sign tracking.
    let mut c = Cmo::new(2);
    let out = feed(&mut c, &[1.0, 2.0, 3.0, 2.0, 2.5]);
    assert_eq!(out[0], None);
    assert_eq!(out[1], None);
    assert_eq!(out[2], Some(100.0)); // gains 1,1
    assert_eq!(out[3], Some(0.0)); // gains 1,1 losses 0,1
    // gains 0.5,1 losses 1,0 -> 100*(0.5 - 1)/(0.5 + 1) = -33.33…
    assert_close(out[4].unwrap(), -100.0 / 3.0);
}

fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn reset_clears_momentum_history() {
    let mut c = Cmo::new(2);
    feed(&mut c, &[1.0, 2.0, 3.0]);
    c.reset();
    assert_eq!(c.value(), None);
    assert_eq!(
        feed(&mut c, &[10.0, 10.0, 10.0, 20.0]),
        [None, None, Some(0.0), Some(100.0)]
    );
}
