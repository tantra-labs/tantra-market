//! Unit tests for `crate::ema`.

mod common;
use common::feed;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Ema, Indicator};

#[test]
#[should_panic(expected = "EMA period must be positive")]
fn zero_period_panics() {
    let _ = Ema::new(0);
}

#[test]
fn seeds_with_the_sma_then_applies_alpha() {
    // Period 3 -> alpha = 2 / (3 + 1) = 0.5; seed = mean(1, 2, 3) = 2.
    let mut e = Ema::new(3);
    assert_eq!(e.period(), 3);
    assert_eq!(
        feed(&mut e, &[dec!(1), dec!(2), dec!(3), dec!(4), dec!(0)]),
        [None, None, Some(dec!(2)), Some(dec!(3)), Some(dec!(1.5))]
    );
    assert_eq!(e.value(), Some(dec!(1.5)));
}

#[test]
fn reset_requires_a_fresh_seed() {
    let mut e = Ema::new(2);
    feed(&mut e, &[dec!(1), dec!(3), dec!(5)]);
    e.reset();
    assert_eq!(e.value(), None);
    assert_eq!(feed(&mut e, &[dec!(10), dec!(20)]), [None, Some(dec!(15))]);
}
