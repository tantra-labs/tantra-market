//! Unit tests for `crate::keltner_channels`.

mod common;
use common::{assert_dec_close, hlc};
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, KeltnerChannels};

#[test]
#[should_panic(expected = "Keltner period must be positive")]
fn zero_period_panics() {
    let _ = KeltnerChannels::new(0, dec!(2));
}

#[test]
#[should_panic(expected = "Keltner mult must be non-negative")]
fn negative_mult_panics() {
    let _ = KeltnerChannels::new(20, dec!(-1));
}

#[test]
fn constant_bars_give_symmetric_bands_around_the_ema() {
    // period 2 -> EMA alpha 2/3, seed = 9; ATR(2) of two 2-unit ranges = 2.
    let mut k = KeltnerChannels::new(2, dec!(2));
    assert_eq!(k.mult(), dec!(2));
    assert_eq!(k.update(&hlc(10.0, 8.0, 9.0)), None);
    let v = k.update(&hlc(10.0, 8.0, 9.0)).unwrap();
    assert_dec_close(v.middle, dec!(9));
    assert_dec_close(v.upper, dec!(13)); // 9 + 2 * 2
    assert_dec_close(v.lower, dec!(5)); // 9 - 2 * 2
}

#[test]
fn update_hlc_matches_update_with_a_bar() {
    let mut by_bar = KeltnerChannels::new(2, dec!(2));
    let mut by_hlc = KeltnerChannels::new(2, dec!(2));
    for (h, l, c) in [
        (dec!(10), dec!(8), dec!(9)),
        (dec!(11), dec!(9), dec!(10)),
        (dec!(12), dec!(10), dec!(11)),
    ] {
        assert_eq!(
            by_hlc.update_hlc(h, l, c),
            by_bar.update(&common::hlc_dec(h, l, c))
        );
    }
}

#[test]
fn reset_clears_both_the_ema_and_the_atr() {
    let mut k = KeltnerChannels::new(2, dec!(2));
    k.update(&hlc(10.0, 8.0, 9.0));
    k.update(&hlc(10.0, 8.0, 9.0));
    k.reset();
    assert_eq!(k.value(), None);
    assert_eq!(k.update(&hlc(10.0, 8.0, 9.0)), None);
}
