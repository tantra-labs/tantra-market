//! Unit tests for `crate::mass_index`.

mod common;
use common::{assert_close, hlc};
use vyapaar_market_indicators::{Indicator, MassIndex};

#[test]
#[should_panic(expected = "Mass Index period must be positive")]
fn zero_period_panics() {
    let _ = MassIndex::new(0);
}

#[test]
fn constant_range_gives_a_constant_mass_index_of_one_per_bar() {
    // With a constant range, both the EMA(9) and its second EMA equal the
    // range, so every ratio is 1 and the 25-ratio sum is 25.
    let mut m = MassIndex::new(25);
    assert_eq!(m.period(), 25);
    let mut last = None;
    for _ in 0..80 {
        last = m.update(&hlc(10.0, 8.0, 9.0));
    }
    assert_close(last.unwrap(), 25.0);
}

#[test]
fn first_value_appears_once_the_ratio_window_fills() {
    // e1 primes at bar 9, e2 at bar 17; 25 ratios then fill at bar 41.
    let mut m = MassIndex::new(25);
    for i in 0..40 {
        assert_eq!(
            m.update(&hlc(10.0, 8.0, 9.0)),
            None,
            "bar {i} should be warmup"
        );
    }
    assert!(m.update(&hlc(10.0, 8.0, 9.0)).is_some());
}

#[test]
fn reset_requires_a_fresh_warmup() {
    let mut m = MassIndex::new(3);
    for _ in 0..60 {
        m.update(&hlc(10.0, 8.0, 9.0));
    }
    m.reset();
    assert_eq!(m.value(), None);
    assert_eq!(m.update(&hlc(10.0, 8.0, 9.0)), None);
}
