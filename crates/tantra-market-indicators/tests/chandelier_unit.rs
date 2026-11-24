//! Unit & conformance tests for `crate::chandelier`.

mod common;
use common::hlc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Atr, ChandelierExit, ChandelierExitValue, Indicator};

fn batch_chandelier(
    bars: &[(Decimal, Decimal, Decimal)],
    period: usize,
    mult: Decimal,
) -> Vec<Option<ChandelierExitValue>> {
    let mut atr = Atr::new(period);
    let mut out = Vec::with_capacity(bars.len());
    for (i, &(h, l, c)) in bars.iter().enumerate() {
        let a = atr.update_hlc(h, l, c);
        if i + 1 < period {
            out.push(None);
            continue;
        }
        let atr_val = a.expect("Atr warms up on period-th bar");
        let window = &bars[i + 1 - period..=i];
        let hh = window.iter().map(|(x, _, _)| *x).max().unwrap();
        let ll = window.iter().map(|(_, x, _)| *x).min().unwrap();
        out.push(Some(ChandelierExitValue {
            long_stop: hh - mult * atr_val,
            short_stop: ll + mult * atr_val,
        }));
    }
    out
}

#[test]
#[should_panic(expected = "ChandelierExit period must be positive")]
fn zero_period_panics() {
    let _ = ChandelierExit::new(0, dec!(3));
}

#[test]
#[should_panic(expected = "ChandelierExit multiplier must be non-negative")]
fn negative_multiplier_panics() {
    let _ = ChandelierExit::new(14, dec!(-1));
}

#[test]
fn golden_vector_reuses_atr_and_rolling_extremes() {
    // Period 2, mult 3:
    // Bar 1: H=10, L=8,  C=9  -> TR=2,   warm-up (None)
    // Bar 2: H=12, L=9,  C=11 -> TR=3,   ATR=(2+3)/2=2.5, HH=12, LL=8
    //   long_stop  = 12 - 3 * 2.5 = 4.5
    //   short_stop = 8  + 3 * 2.5 = 15.5
    // Bar 3: H=11, L=10, C=10.5 -> TR=1, ATR=(2.5*1+1)/2=1.75, HH=12, LL=9
    //   long_stop  = 12 - 3 * 1.75 = 6.75
    //   short_stop = 9  + 3 * 1.75 = 14.25
    let mut ce = ChandelierExit::new(2, dec!(3));
    assert_eq!(ce.period(), 2);
    assert_eq!(ce.mult(), dec!(3));
    assert!(!ce.is_ready());

    assert_eq!(ce.update(&hlc(10.0, 8.0, 9.0)), None);
    assert!(!ce.is_ready());

    let v2 = ce.update(&hlc(12.0, 9.0, 11.0)).unwrap();
    assert!(ce.is_ready());
    assert_eq!(
        v2,
        ChandelierExitValue {
            long_stop: dec!(4.5),
            short_stop: dec!(15.5),
        }
    );
    assert_eq!(ce.value(), Some(v2));

    let v3 = ce.update(&hlc(11.0, 10.0, 10.5)).unwrap();
    assert_eq!(
        v3,
        ChandelierExitValue {
            long_stop: dec!(6.75),
            short_stop: dec!(14.25),
        }
    );
}

#[test]
fn window_slides_out_stale_extremes() {
    let mut ce = ChandelierExit::new(2, dec!(1));
    // Bar 1 sets huge high=100, low=90, TR=10
    assert_eq!(ce.update_hlc(dec!(100), dec!(90), dec!(95)), None);
    // Bar 2: H=96, L=94, C=95 -> TR=2, ATR=6, HH=100, LL=90
    let v2 = ce.update_hlc(dec!(96), dec!(94), dec!(95)).unwrap();
    assert_eq!(v2.long_stop, dec!(94)); // 100 - 6
    assert_eq!(v2.short_stop, dec!(96)); // 90 + 6

    // Bar 3: H=98, L=95, C=97 -> TR=3, ATR=(6+3)/2=4.5, Bar 1 evicted so HH=98, LL=94
    let v3 = ce.update_hlc(dec!(98), dec!(95), dec!(97)).unwrap();
    assert_eq!(v3.long_stop, dec!(93.5)); // 98 - 4.5
    assert_eq!(v3.short_stop, dec!(98.5)); // 94 + 4.5
}

#[test]
fn reset_clears_atr_and_window_state() {
    let mut ce = ChandelierExit::new(2, dec!(2));
    ce.update_hlc(dec!(10), dec!(8), dec!(9));
    ce.update_hlc(dec!(12), dec!(9), dec!(11));
    assert!(ce.is_ready());
    ce.reset();
    assert!(!ce.is_ready());
    assert_eq!(ce.value(), None);
    assert_eq!(ce.update_hlc(dec!(20), dec!(18), dec!(19)), None);
    let v = ce.update_hlc(dec!(22), dec!(20), dec!(21)).unwrap();
    // Without prior close=11: TRs are 2 and 3 (|22-19|), ATR = 2.5, HH=22, LL=18
    assert_eq!(
        v,
        ChandelierExitValue {
            long_stop: dec!(17),
            short_stop: dec!(23),
        }
    );
}

#[test]
fn streaming_matches_batch_reference() {
    let bars: Vec<(Decimal, Decimal, Decimal)> = (0..80)
        .map(|i| {
            let base = Decimal::from(100 + ((i * 19) % 41) - ((i * 7) % 17));
            let lo = base - Decimal::from(1 + (i % 5));
            let hi = base + Decimal::from(1 + ((i * 3) % 7));
            (hi, lo, base)
        })
        .collect();
    for period in [1, 5, 14, 22] {
        for mult in [Decimal::ZERO, dec!(2), dec!(3)] {
            let mut ce = ChandelierExit::new(period, mult);
            let expected = batch_chandelier(&bars, period, mult);
            for (i, &(h, l, c)) in bars.iter().enumerate() {
                assert_eq!(
                    ce.update_hlc(h, l, c),
                    expected[i],
                    "period {period} mult {mult} row {i}"
                );
            }
        }
    }
}
