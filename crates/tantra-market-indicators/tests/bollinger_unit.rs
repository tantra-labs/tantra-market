//! Unit tests for `crate::bollinger`.

mod common;
use common::{assert_dec_close, feed};
use rust_decimal_macros::dec;
use tantra_market_indicators::{BollingerBands, Indicator};

#[test]
#[should_panic(expected = "Bollinger period must be positive")]
fn zero_period_panics() {
    let _ = BollingerBands::new(0, dec!(2));
}

#[test]
#[should_panic(expected = "Bollinger k must be non-negative")]
fn negative_k_panics() {
    let _ = BollingerBands::new(2, dec!(-0.1));
}

#[test]
fn bands_use_population_standard_deviation() {
    let mut b = BollingerBands::new(2, dec!(2));
    assert_eq!((b.period(), b.k()), (2, dec!(2)));
    let v = feed(&mut b, &[dec!(1), dec!(3)])[1].unwrap();
    // mean 2, population std 1.
    assert_dec_close(v.middle, dec!(2));
    assert_dec_close(v.upper, dec!(4));
    assert_dec_close(v.lower, dec!(0));
}

#[test]
fn zero_k_collapses_the_bands_onto_the_middle() {
    let mut b = BollingerBands::new(3, dec!(0));
    let v = feed(&mut b, &[dec!(1), dec!(5), dec!(9)])[2].unwrap();
    assert_eq!((v.upper, v.lower), (v.middle, v.middle));
}

#[test]
fn reset_clears_the_window() {
    let mut b = BollingerBands::new(2, dec!(1));
    feed(&mut b, &[dec!(1), dec!(2)]);
    b.reset();
    assert_eq!(b.value(), None);
    assert_eq!(b.update(dec!(10)), None);
}
