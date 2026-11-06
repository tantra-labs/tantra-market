//! Unit tests for `crate::aroon`.

mod common;
use common::{assert_close, hlc};
use vyapaar_market_indicators::{Aroon, Indicator};

#[test]
#[should_panic(expected = "Aroon period must be positive")]
fn zero_period_panics() {
    let _ = Aroon::new(0);
}

#[test]
fn first_value_appears_after_period_plus_one_bars() {
    let mut a = Aroon::new(2);
    assert_eq!(a.period(), 2);
    assert_eq!(a.update(&hlc(10.0, 8.0, 9.0)), None);
    assert_eq!(a.update(&hlc(12.0, 9.0, 10.0)), None);
    let v = a.update(&hlc(11.0, 10.0, 10.5)).unwrap();
    // Window 10/12/11: highest high at index 1 → up = 50; lowest low 8 at
    // index 0 → down = 0.
    assert_close(v.up, 50.0);
    assert_close(v.down, 0.0);

    let v = a.update(&hlc(13.0, 8.0, 11.0)).unwrap();
    // Window 12/11/13: highest at index 2 → up = 100; lowest 8 at index 2 →
    // down = 100.
    assert_close(v.up, 100.0);
    assert_close(v.down, 100.0);
}

#[test]
fn update_hl_matches_update_with_a_bar() {
    let mut by_bar = Aroon::new(2);
    let mut by_hl = Aroon::new(2);
    for (h, l, c) in [(10.0, 8.0, 9.0), (12.0, 9.0, 10.0), (11.0, 10.0, 10.5)] {
        assert_eq!(by_hl.update_hl(h, l), by_bar.update(&hlc(h, l, c)));
    }
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut a = Aroon::new(2);
    a.update(&hlc(10.0, 8.0, 9.0));
    a.update(&hlc(12.0, 9.0, 10.0));
    a.update(&hlc(11.0, 10.0, 10.5));
    a.reset();
    assert_eq!(a.value(), None);
    assert_eq!(a.update(&hlc(10.0, 8.0, 9.0)), None);
}
