//! Unit tests for `crate::ichimoku`.

mod common;
use common::{assert_dec_close, hlc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Ichimoku, Indicator};

#[test]
#[should_panic(expected = "Ichimoku periods must satisfy tenkan < kijun < senkou_b")]
fn unordered_periods_panic() {
    let _ = Ichimoku::new(26, 9, 52);
}

#[test]
fn constant_bars_put_all_lines_on_the_midpoint() {
    let mut ic = Ichimoku::new(9, 26, 52);
    for _ in 0..51 {
        assert_eq!(ic.update(&hlc(11.0, 9.0, 10.0)), None, "warmup");
    }
    let v = ic.update(&hlc(11.0, 9.0, 10.0)).unwrap();
    assert_dec_close(v.tenkan, dec!(10)); // (11 + 9) / 2
    assert_dec_close(v.kijun, dec!(10));
    assert_dec_close(v.senkou_a, dec!(10));
    assert_dec_close(v.senkou_b, dec!(10));
    assert_dec_close(v.chikou, dec!(10));
}

#[test]
fn rising_bars_lift_senkou_and_chikou() {
    let mut ic = Ichimoku::new(2, 3, 5);
    let mut last = None;
    for i in 1..=8 {
        let h = (i + 2) as f64;
        let l = i as f64;
        let c = (i + 1) as f64;
        last = ic.update(&hlc(h, l, c));
    }
    let v = last.unwrap();
    // Over the last 5 bars (bars 4-8) the highs are 6..10 and the lows 4..8.
    assert_dec_close(v.senkou_b, dec!(7));
    assert_dec_close(v.chikou, dec!(9));
}

#[test]
fn reset_requires_a_fresh_warmup() {
    let mut ic = Ichimoku::new(2, 3, 5);
    for i in 1..=6 {
        let f = Decimal::from(i);
        ic.update(&common::hlc_dec(f + Decimal::ONE, f, f + dec!(0.5)));
    }
    ic.reset();
    assert_eq!(ic.value(), None);
    assert_eq!(ic.update(&hlc(1.0, 0.0, 0.5)), None);
}
