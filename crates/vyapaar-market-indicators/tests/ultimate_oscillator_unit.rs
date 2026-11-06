//! Unit tests for `crate::ultimate_oscillator`.

mod common;
use common::{assert_close, hlc};
use vyapaar_market_indicators::{Indicator, UltimateOscillator};

#[test]
fn constant_bars_give_the_neutral_fifty() {
    // BP = 10 - min(9, 10) = 1; TR = max(11, 10) - min(9, 10) = 2; every
    // average is 0.5, so UO = (4·0.5 + 2·0.5 + 0.5) / 7 · 100 = 50.
    let mut u = UltimateOscillator::new();
    let mut last = None;
    for _ in 0..40 {
        last = u.update(&hlc(11.0, 9.0, 10.0));
    }
    assert_close(last.unwrap(), 50.0);
}

#[test]
fn first_value_appears_after_twenty_nine_bars() {
    // One bar seeds the previous close; 28 more fill the longest horizon.
    let mut u = UltimateOscillator::new();
    assert_eq!(u.update(&hlc(11.0, 9.0, 10.0)), None);
    for i in 0..27 {
        assert_eq!(
            u.update(&hlc(11.0, 9.0, 10.0)),
            None,
            "bar {} should be warmup",
            i + 1
        );
    }
    assert!(u.update(&hlc(11.0, 9.0, 10.0)).is_some());
}

#[test]
fn reset_requires_a_fresh_previous_close() {
    let mut u = UltimateOscillator::new();
    for _ in 0..40 {
        u.update(&hlc(11.0, 9.0, 10.0));
    }
    u.reset();
    assert_eq!(u.value(), None);
    assert_eq!(u.update(&hlc(11.0, 9.0, 10.0)), None);
}
