//! Unit tests for `crate::rma`.

mod common;
use common::{assert_dec_close, feed};
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Rma};

#[test]
#[should_panic(expected = "RMA period must be positive")]
fn zero_period_panics() {
    let _ = Rma::new(0);
}

#[test]
fn seeds_with_sma_then_applies_wilder_smoothing() {
    let mut rma = Rma::new(3);
    assert_eq!(rma.period(), 3);
    let out = feed(&mut rma, &[dec!(1), dec!(2), dec!(3), dec!(6), dec!(9)]);
    assert_eq!(out[0], None);
    assert_eq!(out[1], None);
    // Seed = (1 + 2 + 3) / 3 = 2
    assert_eq!(out[2], Some(dec!(2)));
    // Next = (2 * 2 + 6) / 3 = 10/3
    assert_dec_close(out[3].unwrap(), dec!(10) / dec!(3));
    // Next = ((10/3) * 2 + 9) / 3 = 47/9
    assert_dec_close(out[4].unwrap(), dec!(47) / dec!(9));
    assert_eq!(rma.value(), out[4]);
}

#[test]
fn reset_clears_state() {
    let mut rma = Rma::new(2);
    feed(&mut rma, &[dec!(1), dec!(3), dec!(5)]);
    rma.reset();
    assert_eq!(rma.value(), None);
    assert_eq!(rma.update(dec!(10)), None);
    assert_eq!(rma.update(dec!(20)), Some(dec!(15)));
}
