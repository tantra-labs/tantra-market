//! Unit tests for `crate::roc`.

mod common;
use common::{assert_dec_close, feed};
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Roc};

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
        feed(&mut r, &[dec!(100), dec!(110), dec!(121), dec!(110)]),
        [None, None, Some(dec!(21)), Some(dec!(0))]
    );
    // (121 - 100) / 100 = 21%; (110 - 110) / 110 = 0%.
}

#[test]
fn period_one_roc_is_the_pct_change() {
    let mut r = Roc::new(1);
    assert_eq!(
        feed(&mut r, &[dec!(100), dec!(110)]),
        [None, Some(dec!(10))]
    );
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut r = Roc::new(2);
    feed(&mut r, &[dec!(100), dec!(110), dec!(121)]);
    assert_dec_close(r.value().unwrap(), dec!(21));
    r.reset();
    assert_eq!(r.value(), None);
    assert_eq!(
        feed(&mut r, &[dec!(100), dec!(100), dec!(200)]),
        [None, None, Some(dec!(100))]
    );
}
