//! Unit tests for `crate::parabolic_sar`.

mod common;
use common::{assert_close, hlc};
use vyapaar_market_indicators::{Indicator, ParabolicSar};

#[test]
#[should_panic(expected = "SAR acceleration step must be positive")]
fn zero_step_panics() {
    let _ = ParabolicSar::new(0.0, 0.20);
}

#[test]
#[should_panic(expected = "SAR acceleration cap must be >= step")]
fn cap_below_step_panics() {
    let _ = ParabolicSar::new(0.05, 0.02);
}

#[test]
fn seeds_on_the_second_bar_and_advances_in_an_uptrend() {
    let mut s = ParabolicSar::new(0.02, 0.20);
    assert_eq!(s.update(&hlc(10.0, 8.0, 9.0)), None);
    assert_close(s.update(&hlc(12.0, 9.0, 10.0)).unwrap(), 8.0);
    // sar = 8 + 0.02 * (12 − 8) = 8.08; new high 13 bumps EP and AF to 0.04.
    assert_close(s.update(&hlc(13.0, 11.0, 11.5)).unwrap(), 8.08);
    // sar = 8.08 + 0.04 * (13 − 8.08) = 8.2768.
    assert_close(s.update(&hlc(14.0, 12.0, 13.0)).unwrap(), 8.2768);
}

#[test]
fn a_down_cross_flips_the_trend() {
    let mut s = ParabolicSar::new(0.02, 0.20);
    s.update(&hlc(10.0, 8.0, 9.0));
    s.update(&hlc(12.0, 9.0, 10.0));
    // A bar that closes the gap below the SAR flips to a downtrend.
    let v = s.update(&hlc(9.0, 7.0, 7.5)).unwrap();
    // After the flip, SAR sits at the old EP (12) and AF restarts at 0.02.
    assert_close(v, 12.0);
}

#[test]
fn reset_restarts_at_the_first_bar() {
    let mut s = ParabolicSar::new(0.02, 0.20);
    s.update(&hlc(10.0, 8.0, 9.0));
    s.update(&hlc(12.0, 9.0, 10.0));
    s.reset();
    assert_eq!(s.value(), None);
    assert_eq!(s.update(&hlc(10.0, 8.0, 9.0)), None);
    assert_close(s.update(&hlc(12.0, 9.0, 10.0)).unwrap(), 8.0);
}
