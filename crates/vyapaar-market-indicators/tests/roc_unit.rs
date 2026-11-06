//! Unit tests for `crate::roc`.

mod common;
use common::{assert_close, feed};
use vyapaar_market_indicators::{Indicator, Roc};

#[test]
#[should_panic(expected = "ROC period must be positive")]
fn zero_period_panics() {
    let _ = Roc::new(0);
}

#[test]
fn first_value_appears_after_period_plus_one() {
    let mut r = Roc::new(2);
    assert_eq!(r.period(), 2);
    assert_eq!(
        feed(&mut r, &[100.0, 110.0, 121.0, 110.0]),
        [None, None, Some(21.0), Some(0.0)]
    );
    // (121 − 100) / 100 = 21%; (110 − 110) / 110 = 0%.
}

#[test]
fn period_one_roc_is_the_pct_change() {
    let mut r = Roc::new(1);
    assert_eq!(feed(&mut r, &[100.0, 110.0]), [None, Some(10.0)]);
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut r = Roc::new(2);
    feed(&mut r, &[100.0, 110.0, 121.0]);
    assert_close(r.value().unwrap(), 21.0);
    r.reset();
    assert_eq!(r.value(), None);
    assert_eq!(
        feed(&mut r, &[100.0, 100.0, 200.0]),
        [None, None, Some(100.0)]
    );
}
