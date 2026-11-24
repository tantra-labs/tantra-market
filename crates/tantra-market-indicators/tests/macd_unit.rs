//! Unit tests for `crate::macd`.

mod common;
use common::{assert_dec_close, feed};
use rust_decimal_macros::dec;
use tantra_market_indicators::Macd;

#[test]
#[should_panic(expected = "MACD periods must be positive")]
fn zero_period_panics() {
    let _ = Macd::new(0, 3, 2);
}

#[test]
fn reports_its_periods() {
    let m = Macd::new(2, 4, 3);
    assert_eq!(
        (m.fast_period(), m.slow_period(), m.signal_period()),
        (2, 4, 3)
    );
}

#[test]
fn first_value_arrives_after_slow_plus_signal_minus_one_inputs() {
    let mut m = Macd::new(2, 3, 2);
    let out = feed(&mut m, &[dec!(1), dec!(2), dec!(3), dec!(4), dec!(5)]);
    let first = out.iter().position(Option::is_some).unwrap();
    assert_eq!(first, 3 + 2 - 2);
}

#[test]
fn constant_input_converges_to_zero() {
    let mut m = Macd::new(2, 3, 2);
    let v = feed(&mut m, &[dec!(7); 6])
        .last()
        .copied()
        .flatten()
        .unwrap();
    assert_dec_close(v.macd, dec!(0));
    assert_dec_close(v.signal, dec!(0));
    assert_dec_close(v.histogram, dec!(0));
}
