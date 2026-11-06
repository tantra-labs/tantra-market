//! Unit tests for `crate::cci`.

mod common;
use common::{assert_close, hlc};
use vyapaar_market_indicators::{Cci, Indicator};

#[test]
#[should_panic(expected = "CCI period must be positive")]
fn zero_period_panics() {
    let _ = Cci::new(0);
}

#[test]
fn golden_values_for_a_linear_run() {
    let mut c = Cci::new(3);
    assert_eq!(c.period(), 3);
    // TPs: 9, 10, 11, 10.
    assert_eq!(c.update(&hlc(10.0, 8.0, 9.0)), None);
    assert_eq!(c.update(&hlc(11.0, 9.0, 10.0)), None);
    // sma = 10, mean deviation = 2/3 → (11 − 10) / (0.015 · 2/3) = 100.
    assert_close(c.update(&hlc(12.0, 10.0, 11.0)).unwrap(), 100.0);
    // sma = 31/3, md = 4/9 → (10 − 31/3) / (0.015 · 4/9) = −50.
    assert_close(c.update(&hlc(11.0, 9.0, 10.0)).unwrap(), -50.0);
}

#[test]
fn constant_bars_give_zero() {
    let mut c = Cci::new(3);
    let mut last = None;
    for _ in 0..10 {
        last = c.update(&hlc(10.0, 8.0, 9.0));
    }
    assert_close(last.unwrap(), 0.0);
}

#[test]
fn update_hlc_matches_update_with_a_bar() {
    let mut by_bar = Cci::new(3);
    let mut by_hlc = Cci::new(3);
    for (h, l, c) in [(10.0, 8.0, 9.0), (11.0, 9.0, 10.0), (12.0, 10.0, 11.0)] {
        assert_eq!(by_hlc.update_hlc(h, l, c), by_bar.update(&hlc(h, l, c)));
    }
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut c = Cci::new(3);
    c.update(&hlc(10.0, 8.0, 9.0));
    c.update(&hlc(11.0, 9.0, 10.0));
    c.update(&hlc(12.0, 10.0, 11.0));
    c.reset();
    assert_eq!(c.value(), None);
    assert_eq!(c.update(&hlc(10.0, 8.0, 9.0)), None);
}
