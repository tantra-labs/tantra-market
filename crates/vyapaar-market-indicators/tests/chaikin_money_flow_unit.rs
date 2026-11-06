//! Unit tests for `crate::chaikin_money_flow`.

mod common;
use common::{assert_close, hlcv};
use vyapaar_market_indicators::{ChaikinMoneyFlow, Indicator};

#[test]
#[should_panic(expected = "CMF period must be positive")]
fn zero_period_panics() {
    let _ = ChaikinMoneyFlow::new(0);
}

#[test]
fn golden_values_for_a_known_flow_sequence() {
    let mut c = ChaikinMoneyFlow::new(2);
    // bar1: mfm 0, flow 0. bar2: mfm 1, flow 10 (volume 10).
    assert_eq!(c.update(&hlcv(10.0, 8.0, 9.0, 10.0)), None);
    // flows [0, 10], volumes [10, 10] → 10 / 20 = 0.5.
    assert_close(c.update(&hlcv(11.0, 9.0, 11.0, 10.0)).unwrap(), 0.5);
    // bar3: mfm -1, flow -10 → flows [10, -10] → 0.
    assert_close(c.update(&hlcv(12.0, 10.0, 10.0, 10.0)).unwrap(), 0.0);
}

#[test]
fn output_is_bounded_between_minus_one_and_one() {
    let mut c = ChaikinMoneyFlow::new(3);
    let mut last = None;
    for i in 0..10 {
        let h = 10.0 + i as f64 * 0.5;
        last = c.update(&hlcv(h, 9.0, 9.5, 5.0));
    }
    assert!(last.unwrap().abs() <= 1.0);
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut c = ChaikinMoneyFlow::new(2);
    c.update(&hlcv(10.0, 8.0, 9.0, 5.0));
    c.update(&hlcv(11.0, 9.0, 11.0, 5.0));
    c.reset();
    assert_eq!(c.value(), None);
    assert_eq!(c.update(&hlcv(10.0, 8.0, 9.0, 5.0)), None);
}
