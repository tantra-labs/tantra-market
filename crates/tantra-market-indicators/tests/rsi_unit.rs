//! Unit tests for `crate::rsi`.

mod common;
use common::{assert_dec_close, feed};
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Rsi};

#[test]
#[should_panic(expected = "RSI period must be positive")]
fn zero_period_panics() {
    let _ = Rsi::new(0);
}

#[test]
fn seeds_from_simple_averages_then_uses_wilder_smoothing() {
    let mut r = Rsi::new(2);
    assert_eq!(r.period(), 2);
    let out = feed(&mut r, &[dec!(1), dec!(2), dec!(1), dec!(3)]);
    assert_eq!(out[..2], [None, None]);
    // Seed: avg gain 0.5, avg loss 0.5 -> RSI 50.
    assert_dec_close(out[2].unwrap(), dec!(50));
    // Wilder: gain (0.5 + 2) / 2 = 1.25, loss (0.5 + 0) / 2 = 0.25 -> RS 5.
    assert_dec_close(out[3].unwrap(), dec!(100) - dec!(100) / dec!(6));
    assert_eq!(r.value(), out[3]);
}

#[test]
fn reset_forgets_the_previous_price() {
    let mut r = Rsi::new(1);
    feed(&mut r, &[dec!(1), dec!(2)]);
    assert!(r.is_ready());
    r.reset();
    assert!(!r.is_ready());
    // First input after reset only records the price.
    assert_eq!(r.update(dec!(5)), None);
    assert_eq!(r.update(dec!(4)), Some(dec!(0)));
}
