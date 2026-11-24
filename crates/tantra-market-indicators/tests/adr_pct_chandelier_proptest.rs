//! Proptest invariants for [`AdrPct`] and [`ChandelierExit`]:
//! - `ADR% >= 0` for valid positive OHLC bars (`high >= low > 0`)
//! - `ChandelierExit` long stop `<= highest high` over the period
//! - `ChandelierExit` short stop `>= lowest low` over the period
//! - Streaming updates match batch recompute for both indicators

use proptest::collection::vec as prop_vec;
use proptest::prelude::*;
use rust_decimal::Decimal;
use tantra_market_indicators::{AdrPct, Atr, ChandelierExit, ChandelierExitValue, Indicator};

/// `(high, low, close)` in ticks (cents), with `low > 0` and `low <= close <= high`.
fn hlc_tuple() -> impl Strategy<Value = (Decimal, Decimal, Decimal)> {
    (1i64..=10_000i64, 0i64..=5_000i64, 0i64..=5_000i64).prop_map(|(lo, span, c_off)| {
        let high_t = lo + span;
        let close_t = if span == 0 {
            lo
        } else {
            lo + (c_off % (span + 1))
        };
        (
            Decimal::new(high_t, 2),
            Decimal::new(lo, 2),
            Decimal::new(close_t, 2),
        )
    })
}

proptest! {
    #[test]
    fn adr_pct_non_negative_and_matches_batch(
        period in 1usize..=25usize,
        bars in prop_vec(hlc_tuple(), 1..=60),
    ) {
        let mut adr = AdrPct::new(period);
        let n = Decimal::from(period);
        for (i, &(h, l, _)) in bars.iter().enumerate() {
            let got = adr.update_hl(h, l);
            if i + 1 < period {
                prop_assert_eq!(got, None);
                prop_assert!(!adr.is_ready());
            } else {
                let v = got.expect("ADR% must emit once warmed up");
                prop_assert!(v >= Decimal::ZERO, "ADR% must be >= 0 for H >= L > 0: {v}");
                let sum: Decimal = bars[i + 1 - period..=i]
                    .iter()
                    .map(|(hi, lo, _)| *hi / *lo)
                    .sum();
                let batch = (sum / n - Decimal::ONE) * Decimal::ONE_HUNDRED;
                prop_assert_eq!(v, batch, "streaming != batch at row {}", i);
            }
        }
    }

    #[test]
    fn chandelier_stops_bounded_by_extremes_and_match_batch(
        period in 1usize..=25usize,
        mult_tenths in 0i64..=50i64,
        bars in prop_vec(hlc_tuple(), 1..=60),
    ) {
        let mult = Decimal::new(mult_tenths, 1);
        let mut ce = ChandelierExit::new(period, mult);
        let mut ref_atr = Atr::new(period);

        for (i, &(h, l, c)) in bars.iter().enumerate() {
            let atr = ref_atr.update_hlc(h, l, c);
            let got = ce.update_hlc(h, l, c);
            if i + 1 < period {
                prop_assert_eq!(got, None);
                prop_assert!(!ce.is_ready());
            } else {
                let v = got.expect("ChandelierExit must emit once warmed up");
                let window = &bars[i + 1 - period..=i];
                let hh = window.iter().map(|(x, _, _)| *x).max().unwrap();
                let ll = window.iter().map(|(_, x, _)| *x).min().unwrap();

                prop_assert!(
                    v.long_stop <= hh,
                    "long_stop {} > highest_high {}",
                    v.long_stop,
                    hh
                );
                prop_assert!(
                    v.short_stop >= ll,
                    "short_stop {} < lowest_low {}",
                    v.short_stop,
                    ll
                );

                let atr_val = atr.unwrap();
                let batch = ChandelierExitValue {
                    long_stop: hh - mult * atr_val,
                    short_stop: ll + mult * atr_val,
                };
                prop_assert_eq!(v, batch, "streaming != batch at row {}", i);
            }
        }
    }
}
