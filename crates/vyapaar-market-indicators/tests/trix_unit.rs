//! Unit tests for `crate::trix`.

mod common;
use common::{assert_close, feed};
use vyapaar_market_indicators::{Indicator, Trix};

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
        feed(&mut t, &[1.0, 2.0, 4.0]),
        [None, Some(100.0), Some(100.0)]
    );
}

#[test]
fn constant_price_gives_zero_trix() {
    let mut t = Trix::new(3);
    let out = feed(&mut t, &[10.0; 30]);
    // 3*3 = 9 inputs prime the triple EMA; input 8 (index 7) is the first
    // percentage change, so index 6 is still warmup.
    assert_eq!(out[0], None);
    assert_eq!(out[6], None);
    assert_eq!(out[7], Some(0.0));
    for v in out.iter().flatten() {
        assert_close(*v, 0.0);
    }
}

#[test]
fn reset_requires_a_fresh_triple_seed() {
    let mut t = Trix::new(1);
    feed(&mut t, &[1.0, 2.0]);
    t.reset();
    assert_eq!(t.value(), None);
    assert_eq!(feed(&mut t, &[5.0, 10.0]), [None, Some(100.0)]);
}
