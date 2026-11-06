//! Unit tests for `crate::keltner_channels`.

mod common;
use common::{assert_close, hlc};
use vyapaar_market_indicators::{Indicator, KeltnerChannels};

#[test]
#[should_panic(expected = "Keltner period must be positive")]
fn zero_period_panics() {
    let _ = KeltnerChannels::new(0, 2.0);
}

#[test]
#[should_panic(expected = "Keltner mult must be non-negative")]
fn negative_mult_panics() {
    let _ = KeltnerChannels::new(20, -1.0);
}

#[test]
fn constant_bars_give_symmetric_bands_around_the_ema() {
    // period 2 -> EMA alpha 2/3, seed = 9; ATR(2) of two 2-unit ranges = 2.
    let mut k = KeltnerChannels::new(2, 2.0);
    assert_eq!(k.mult(), 2.0);
    assert_eq!(k.update(&hlc(10.0, 8.0, 9.0)), None);
    let v = k.update(&hlc(10.0, 8.0, 9.0)).unwrap();
    assert_close(v.middle, 9.0);
    assert_close(v.upper, 13.0); // 9 + 2 * 2
    assert_close(v.lower, 5.0); // 9 - 2 * 2
}

#[test]
fn update_hlc_matches_update_with_a_bar() {
    let mut by_bar = KeltnerChannels::new(2, 2.0);
    let mut by_hlc = KeltnerChannels::new(2, 2.0);
    for (h, l, c) in [(10.0, 8.0, 9.0), (11.0, 9.0, 10.0), (12.0, 10.0, 11.0)] {
        assert_eq!(by_hlc.update_hlc(h, l, c), by_bar.update(&hlc(h, l, c)));
    }
}

#[test]
fn reset_clears_both_the_ema_and_the_atr() {
    let mut k = KeltnerChannels::new(2, 2.0);
    k.update(&hlc(10.0, 8.0, 9.0));
    k.update(&hlc(10.0, 8.0, 9.0));
    k.reset();
    assert_eq!(k.value(), None);
    assert_eq!(k.update(&hlc(10.0, 8.0, 9.0)), None);
}
