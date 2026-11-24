//! Property tests for Decimal-native indicators (gap 0009):
//! bounds, ordering invariants, zero-float determinism, and non-finite shim rejection.

mod common;
use common::hlcv_dec;
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{
    BollingerBands, ChaikinMoneyFlow, Donchian, Ema, FisherTransform, HullMa, Indicator, Kst, Macd,
    Mfi, Roc, Rsi, Sma, Stochastic, Trix, WilliamsR,
};

fn dec_prices(len: usize) -> impl Strategy<Value = Vec<Decimal>> {
    proptest::collection::vec(100_i64..50_000, len..len + 1)
        .prop_map(|v| v.into_iter().map(|n| Decimal::new(n, 2)).collect())
}

proptest! {
    #[test]
    fn sma_and_ema_stay_within_min_max_of_series(prices in dec_prices(40)) {
        let lo = *prices.iter().min().unwrap();
        let hi = *prices.iter().max().unwrap();
        let mut sma = Sma::new(5);
        let mut ema = Ema::new(5);
        for &p in &prices {
            if let Some(v) = sma.update(p) {
                prop_assert!(v >= lo && v <= hi, "sma {v} outside [{lo}, {hi}]");
            }
            if let Some(v) = ema.update(p) {
                prop_assert!(v >= lo && v <= hi, "ema {v} outside [{lo}, {hi}]");
            }
        }
    }

    #[test]
    fn bollinger_bands_maintain_lower_le_middle_le_upper(prices in dec_prices(30)) {
        let mut bb = BollingerBands::new(10, dec!(2));
        for &p in &prices {
            if let Some(v) = bb.update(p) {
                prop_assert!(v.lower <= v.middle, "lower {} > middle {}", v.lower, v.middle);
                prop_assert!(v.middle <= v.upper, "middle {} > upper {}", v.middle, v.upper);
            }
        }
    }

    #[test]
    fn bounded_oscillators_respect_range_invariants(
        bars in proptest::collection::vec((100_i64..10_000, 0_i64..500, 0_i64..100), 30..40)
    ) {
        let mut rsi = Rsi::new(7);
        let mut mfi = Mfi::new(7);
        let mut cmf = ChaikinMoneyFlow::new(7);
        let mut stoch = Stochastic::new(7, 3);
        let mut wr = WilliamsR::new(7);
        let mut donchian = Donchian::new(7);

        for (base, spread, close_pct) in bars {
            let low = Decimal::new(base, 2);
            let high = low + Decimal::new(spread, 2);
            let close = low + (high - low) * Decimal::new(close_pct, 2);
            let bar = hlcv_dec(high, low, close, dec!(100));

            if let Some(v) = rsi.update(close) {
                prop_assert!(v >= Decimal::ZERO && v <= dec!(100));
            }
            if let Some(v) = mfi.update(&bar) {
                prop_assert!(v >= Decimal::ZERO && v <= dec!(100));
            }
            if let Some(v) = cmf.update(&bar) {
                prop_assert!(v >= dec!(-1) && v <= Decimal::ONE);
            }
            if let Some(v) = stoch.update(&bar) {
                prop_assert!(v.k >= Decimal::ZERO && v.k <= dec!(100));
            }
            if let Some(v) = wr.update(&bar) {
                prop_assert!(v >= dec!(-100) && v <= Decimal::ZERO);
            }
            if let Some(v) = donchian.update(&bar) {
                prop_assert!(v.lower <= v.middle && v.middle <= v.upper);
            }
        }
    }
}

#[test]
fn all_f64_presentation_shims_reject_non_finite_without_mutating_state() {
    let mut sma = Sma::new(2);
    let mut ema = Ema::new(2);
    let mut rsi = Rsi::new(2);
    let mut roc = Roc::new(1);
    let mut hull = HullMa::new(2);
    let mut fisher = FisherTransform::new(2);
    let mut kst = Kst::new();
    let mut trix = Trix::new(1);
    let mut macd = Macd::new(2, 3, 2);
    let _ = &mut macd; // no f64 shim on Macd

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(sma.update_f64(bad), None);
        assert_eq!(ema.update_f64(bad), None);
        assert_eq!(rsi.update_f64(bad), None);
        assert_eq!(roc.update_f64(bad), None);
        assert_eq!(hull.update_f64(bad), None);
        assert_eq!(fisher.update_f64(bad), None);
        assert_eq!(kst.update_f64(bad), None);
        assert_eq!(trix.update_f64(bad), None);
    }

    assert!(!sma.is_ready());
    assert!(!ema.is_ready());
    assert!(!rsi.is_ready());
    assert!(!roc.is_ready());
    assert!(!hull.is_ready());
    assert!(!fisher.is_ready());
    assert!(!kst.is_ready());
    assert!(!trix.is_ready());
}
