//! Unit tests for `crate::vortex`.

mod common;
use common::{assert_close, hlc};
use vyapaar_market_indicators::{Indicator, Vortex};

#[test]
#[should_panic(expected = "Vortex period must be positive")]
fn zero_period_panics() {
    let _ = Vortex::new(0);
}

#[test]
fn golden_values_for_a_known_sequence() {
    let mut v = Vortex::new(2);
    assert_eq!(v.period(), 2);
    assert_eq!(v.update(&hlc(10.0, 8.0, 9.0)), None);
    assert_eq!(v.update(&hlc(11.0, 9.0, 10.0)), None);
    // bar3: TR 3, plus |12-9|=3, minus |9-11|=2 → VI+ = 6/5 = 1.2, VI− = 3/5.
    let out = v.update(&hlc(12.0, 9.0, 11.0)).unwrap();
    assert_close(out.vi_plus, 1.2);
    assert_close(out.vi_minus, 0.6);
    // bar4: TR 3, plus |11-9|=2, minus |8-12|=4 → VI+ = 5/6, VI− = 1.
    let out = v.update(&hlc(11.0, 8.0, 10.0)).unwrap();
    assert_close(out.vi_plus, 5.0 / 6.0);
    assert_close(out.vi_minus, 1.0);
}

#[test]
fn update_hlc_matches_update_with_a_bar() {
    let mut by_bar = Vortex::new(2);
    let mut by_hlc = Vortex::new(2);
    for (h, l, c) in [(10.0, 8.0, 9.0), (11.0, 9.0, 10.0), (12.0, 9.0, 11.0)] {
        assert_eq!(by_hlc.update_hlc(h, l, c), by_bar.update(&hlc(h, l, c)));
    }
}

#[test]
fn reset_requires_a_fresh_previous_bar() {
    let mut v = Vortex::new(2);
    v.update(&hlc(10.0, 8.0, 9.0));
    v.update(&hlc(11.0, 9.0, 10.0));
    v.update(&hlc(12.0, 9.0, 11.0));
    v.reset();
    assert_eq!(v.value(), None);
    assert_eq!(v.update(&hlc(10.0, 8.0, 9.0)), None);
}
