//! Unit tests for `crate::adx`.

mod common;
use common::{assert_dec_close, hlc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Adx, Indicator};

#[test]
#[should_panic(expected = "ADX period must be positive")]
fn zero_period_panics() {
    let _ = Adx::new(0);
}

#[test]
fn warmup_and_steady_uptrend_produce_positive_di_and_high_adx() {
    let mut adx = Adx::new(3);
    assert_eq!(adx.period(), 3);
    // 1 baseline + 3 bars to compute 1st +DI/-DI/DX (bar 4), + 2 more DXs to yield 1st ADX (bar 6).
    for i in 0..5 {
        let b = hlc(
            100.0 + i as f64 * 2.0,
            99.0 + i as f64 * 2.0,
            99.5 + i as f64 * 2.0,
        );
        assert_eq!(adx.update(&b), None, "warmup at bar {i}");
    }
    let first = adx
        .update(&hlc(110.0, 109.0, 109.5))
        .expect("ADX primes at 2 * period bars");
    assert!(first > Decimal::ZERO);
    // +DM = 2.0, TR = 2.5 => +DI = 100 * 2 / 2.5 = 80, -DI = 0, DX = 100.
    assert_dec_close(adx.plus_di().unwrap(), dec!(80));
    assert_dec_close(adx.minus_di().unwrap(), dec!(0));
    assert_dec_close(adx.dx().unwrap(), dec!(100));
    assert_dec_close(first, dec!(100));
    assert_eq!(adx.value(), Some(first));
}

#[test]
fn steady_downtrend_dominates_minus_di() {
    let mut adx = Adx::new(3);
    let mut last = None;
    for i in 0..10 {
        let p = 200.0 - i as f64 * 3.0;
        last = adx.update(&hlc(p + 1.0, p, p + 0.5));
    }
    let val = last.unwrap();
    assert!(adx.minus_di().unwrap() > adx.plus_di().unwrap());
    assert!(val > dec!(50));
}

#[test]
fn reset_clears_all_accumulators() {
    let mut adx = Adx::new(2);
    for i in 0..6 {
        let p = 100.0 + i as f64;
        adx.update(&hlc(p + 1.0, p - 1.0, p));
    }
    assert!(adx.is_ready());
    adx.reset();
    assert!(!adx.is_ready());
    assert_eq!(adx.value(), None);
    assert_eq!(adx.plus_di(), None);
    assert_eq!(adx.minus_di(), None);
    assert_eq!(adx.dx(), None);
    assert_eq!(adx.update(&hlc(101.0, 99.0, 100.0)), None);
}
