//! Unit tests for `crate::mfi`.

mod common;
use common::{assert_close, hlcv};
use vyapaar_market_indicators::{Indicator, Mfi};

#[test]
#[should_panic(expected = "MFI period must be positive")]
fn zero_period_panics() {
    let _ = Mfi::new(0);
}

#[test]
fn all_positive_flow_is_one_hundred() {
    let mut m = Mfi::new(2);
    // bar1: mfm = ((9-8) - (10-9)) / 2 = 0 → flow 0.
    assert_eq!(m.update(&hlcv(10.0, 8.0, 9.0, 10.0)), None);
    // bar2: mfm = ((11-9) - (11-11)) / 2 = 1 → flow 10 → MFI 100.
    assert_close(m.update(&hlcv(11.0, 9.0, 11.0, 10.0)).unwrap(), 100.0);
    // bar3: mfm = ((10-10) - (12-10)) / 2 = -1 → flow -10 → MFI 50.
    assert_close(m.update(&hlcv(12.0, 10.0, 10.0, 10.0)).unwrap(), 50.0);
}

#[test]
fn zero_flow_is_neutral() {
    let mut m = Mfi::new(2);
    let mut last = None;
    for _ in 0..10 {
        last = m.update(&hlcv(10.0, 8.0, 9.0, 5.0));
    }
    assert_close(last.unwrap(), 50.0);
}

#[test]
fn multiplier_is_zero_for_a_zero_range_bar() {
    assert_eq!(Mfi::multiplier(10.0, 10.0, 10.0), 0.0);
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut m = Mfi::new(2);
    m.update(&hlcv(10.0, 8.0, 9.0, 5.0));
    m.update(&hlcv(11.0, 9.0, 11.0, 5.0));
    m.reset();
    assert_eq!(m.value(), None);
    assert_eq!(m.update(&hlcv(10.0, 8.0, 9.0, 5.0)), None);
}
