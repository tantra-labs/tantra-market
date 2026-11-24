//! Behavioural tests for every indicator.

use rust_decimal::{Decimal, MathematicalOps};
use rust_decimal_macros::dec;
use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
use tantra_market_indicators::{
    Atr, BollingerBands, BollingerValue, Ema, Indicator, Macd, Rsi, Sma,
};

fn bar(high: Decimal, low: Decimal, close: Decimal) -> Bar {
    Bar {
        instrument: InstrumentId::try_new(Exchange::Nse, "X").unwrap(),
        open: Price::try_new(close).unwrap(),
        high: Price::try_new(high).unwrap(),
        low: Price::try_new(low).unwrap(),
        close: Price::try_new(close).unwrap(),
        volume: Quantity::from_u64(1),
        timestamp: chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("valid ts"),
    }
}

// --- SMA ---

#[test]
fn sma_returns_none_until_primed() {
    let mut s = Sma::new(3);
    assert_eq!(s.update(dec!(1)), None);
    assert_eq!(s.update(dec!(2)), None);
    assert_eq!(s.update(dec!(3)), Some(dec!(2)));
    assert!(s.is_ready());
}

#[test]
fn sma_rolling_average() {
    let mut s = Sma::new(3);
    let inputs = [dec!(1), dec!(2), dec!(3), dec!(4), dec!(5)];
    let expected: Vec<Option<Decimal>> =
        vec![None, None, Some(dec!(2)), Some(dec!(3)), Some(dec!(4))];
    for (i, x) in inputs.iter().enumerate() {
        assert_eq!(s.update(*x), expected[i], "at index {i}");
    }
}

#[test]
fn sma_reset_clears_state() {
    let mut s = Sma::new(3);
    s.update(dec!(1));
    s.update(dec!(2));
    s.update(dec!(3));
    assert!(s.is_ready());
    s.reset();
    assert!(!s.is_ready());
    assert_eq!(s.update(dec!(10)), None);
}

// --- EMA ---

#[test]
fn ema_seeds_from_sma() {
    let mut e = Ema::new(3);
    assert_eq!(e.update(dec!(1)), None);
    assert_eq!(e.update(dec!(2)), None);
    assert_eq!(e.update(dec!(3)), Some(dec!(2)));
}

#[test]
fn ema_smooths_after_seed() {
    let mut e = Ema::new(3); // alpha = 0.5
    for x in [dec!(1), dec!(2), dec!(3)] {
        e.update(x);
    }
    assert_eq!(e.update(dec!(4)), Some(dec!(3)));
    assert_eq!(e.update(dec!(5)), Some(dec!(4)));
}

#[test]
fn ema_reset_clears_state() {
    let mut e = Ema::new(3);
    for x in [dec!(1), dec!(2), dec!(3)] {
        e.update(x);
    }
    e.reset();
    assert!(!e.is_ready());
    assert_eq!(e.update(dec!(1)), None);
}

// --- RSI ---

#[test]
fn rsi_all_up_is_100() {
    let mut r = Rsi::new(3);
    for p in [dec!(1), dec!(2), dec!(3), dec!(4)] {
        r.update(p);
    }
    let v = r.value().unwrap();
    assert!((v - dec!(100)).abs() < dec!(1e-9), "expected 100, got {v}");
}

#[test]
fn rsi_all_down_is_zero() {
    let mut r = Rsi::new(3);
    for p in [dec!(4), dec!(3), dec!(2), dec!(1)] {
        r.update(p);
    }
    let v = r.value().unwrap();
    assert!(v.abs() < dec!(1e-9), "expected 0, got {v}");
}

#[test]
fn rsi_flat_is_neutral_when_no_loss() {
    // With all gains = 0 and all losses = 0, avg_loss == 0, so we return 100.
    // This is an edge case; document the behaviour rather than fight it.
    let mut r = Rsi::new(3);
    for p in [dec!(5), dec!(5), dec!(5), dec!(5)] {
        r.update(p);
    }
    let v = r.value().unwrap();
    assert!((v - dec!(100)).abs() < dec!(1e-9));
}

#[test]
fn rsi_requires_period_plus_one_inputs() {
    let mut r = Rsi::new(3);
    assert_eq!(r.update(dec!(1)), None); // first price sets baseline
    assert_eq!(r.update(dec!(2)), None);
    assert_eq!(r.update(dec!(3)), None);
    assert_eq!(r.update(dec!(4)), Some(dec!(100)));
}

// --- ATR ---

#[test]
fn atr_first_bar_uses_high_low() {
    let mut a = Atr::new(2);
    assert_eq!(a.update(&bar(dec!(10), dec!(8), dec!(9))), None); // tr = 2
    // Second bar: high=11, low=9, prev_close=9 -> tr = max(2, 2, 0) = 2
    assert_eq!(a.update(&bar(dec!(11), dec!(9), dec!(10))), Some(dec!(2)));
}

#[test]
fn atr_accounts_for_gaps() {
    let mut a = Atr::new(1);
    a.update(&bar(dec!(10), dec!(9), dec!(9.5))); // tr = 1
    // Gap up: prev_close=9.5, high=20, low=15 -> tr = max(5, 10.5, 5.5) = 10.5
    let v = a.update(&bar(dec!(20), dec!(15), dec!(19))).unwrap();
    assert_eq!(v, dec!(10.5));
}

// --- Bollinger ---

#[test]
fn bollinger_returns_none_until_primed() {
    let mut bb = BollingerBands::new(3, dec!(2));
    assert_eq!(bb.update(dec!(1)), None);
    assert_eq!(bb.update(dec!(2)), None);
    assert!(bb.update(dec!(3)).is_some());
}

#[test]
fn bollinger_constant_series_has_zero_width() {
    let mut bb = BollingerBands::new(5, dec!(2));
    for _ in 0..5 {
        bb.update(dec!(10));
    }
    let v = bb.value().unwrap();
    assert_eq!(v.middle, dec!(10));
    assert_eq!(v.upper, dec!(10));
    assert_eq!(v.lower, dec!(10));
}

#[test]
fn bollinger_bands_bracket_middle() {
    let mut bb = BollingerBands::new(5, dec!(2));
    for x in [dec!(1), dec!(2), dec!(3), dec!(4), dec!(5)] {
        bb.update(x);
    }
    let v: BollingerValue = bb.value().unwrap();
    assert_eq!(v.middle, dec!(3));
    let expected_std = dec!(2).sqrt().unwrap();
    assert!((v.upper - (dec!(3) + dec!(2) * expected_std)).abs() < dec!(1e-9));
    assert!((v.lower - (dec!(3) - dec!(2) * expected_std)).abs() < dec!(1e-9));
}

// --- MACD ---

#[test]
#[should_panic(expected = "fast period must be less than slow period")]
fn macd_rejects_fast_ge_slow() {
    let _ = Macd::new(26, 12, 9);
}

#[test]
fn macd_none_until_all_emas_primed() {
    let mut m = Macd::new(2, 4, 2);
    // Slow (4) + signal (2) -> needs at least 5 values before Some.
    for i in 1..=4 {
        assert_eq!(m.update(Decimal::from(i)), None, "at step {i}");
    }
    assert!(m.update(dec!(5)).is_some());
}

#[test]
fn macd_histogram_equals_macd_minus_signal() {
    let mut m = Macd::new(2, 4, 2);
    for x in 1..=20 {
        m.update(Decimal::from(x));
    }
    let v = m.value().unwrap();
    assert!((v.histogram - (v.macd - v.signal)).abs() < dec!(1e-12));
    assert!(v.macd > Decimal::ZERO);
}

#[test]
fn macd_reset_clears_state() {
    let mut m = Macd::new(2, 4, 2);
    for x in 1..=20 {
        m.update(Decimal::from(x));
    }
    assert!(m.is_ready());
    m.reset();
    assert!(!m.is_ready());
    assert_eq!(m.update(dec!(1)), None);
}
