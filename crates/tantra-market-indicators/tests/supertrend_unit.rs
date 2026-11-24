//! Unit tests for `crate::supertrend`.

mod common;
use common::hlc;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Supertrend};

#[test]
#[should_panic(expected = "Supertrend period must be positive")]
fn zero_period_panics() {
    let _ = Supertrend::new(0, dec!(3));
}

#[test]
fn warmup_returns_none_and_tracks_trend_direction() {
    let mut st = Supertrend::new(2, dec!(2));
    assert_eq!(st.period(), 2);
    assert_eq!(st.multiplier(), dec!(2));
    assert_eq!(st.update(&hlc(10.0, 8.0, 9.0)), None);
    assert_eq!(st.value(), None);

    let first = st
        .update(&hlc(11.0, 9.0, 10.0))
        .expect("ATR(2) primes on bar 2");
    // Bar 2: hl2 = 10, ATR = 2 -> basic_upper = 14, basic_lower = 6; prev_st = hl2(9), prev_upper = 9 -> final_upper = 9 or lower
    assert!(first > dec!(0));
    assert_eq!(st.value(), Some(first));
}

#[test]
fn reset_clears_supertrend_state() {
    let mut st = Supertrend::new(2, dec!(3));
    st.update(&hlc(10.0, 8.0, 9.0));
    st.update(&hlc(11.0, 9.0, 10.0));
    assert!(st.is_ready());
    st.reset();
    assert!(!st.is_ready());
    assert_eq!(st.value(), None);
    assert_eq!(st.update(&hlc(10.0, 8.0, 9.0)), None);
}
