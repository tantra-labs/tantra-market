//! Unit tests for `crate::trix`.

mod common;
use common::{assert_dec_close, feed};
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Trix};

#[test]
#[should_panic(expected = "TRIX period must be positive")]
fn zero_period_panics() {
    let _ = Trix::new(0);
}

#[test]
fn period_one_trix_is_the_pct_change_of_price() {
    // With period 1 the EMA alpha is 1, so the triple EMA is the price itself
    // and TRIX is the period-over-period percentage change.
    let mut t = Trix::new(1);
    assert_eq!(t.period(), 1);
    assert_eq!(
        feed(&mut t, &[dec!(1), dec!(2), dec!(4)]),
        [None, Some(dec!(100)), Some(dec!(100))]
    );
}

#[test]
fn constant_price_gives_zero_trix() {
    let mut t = Trix::new(3);
    let out = feed(&mut t, &[dec!(10); 30]);
    // 3*3 = 9 inputs prime the triple EMA; input 8 (index 7) is the first
    // percentage change, so index 6 is still warmup.
    assert_eq!(out[0], None);
    assert_eq!(out[6], None);
    assert_eq!(out[7], Some(dec!(0)));
    for v in out.iter().flatten() {
        assert_dec_close(*v, dec!(0));
    }
}

#[test]
fn reset_requires_a_fresh_triple_seed() {
    let mut t = Trix::new(1);
    feed(&mut t, &[dec!(1), dec!(2)]);
    t.reset();
    assert_eq!(t.value(), None);
    assert_eq!(feed(&mut t, &[dec!(5), dec!(10)]), [None, Some(dec!(100))]);
}
