//! Unit tests for `crate::donchian`.

mod common;
use common::{assert_dec_close, hlc, hlc_dec};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Donchian, Indicator};

#[test]
#[should_panic(expected = "Donchian period must be positive")]
fn zero_period_panics() {
    let _ = Donchian::new(0);
}

#[test]
fn reports_its_period_and_readiness() {
    let mut d = Donchian::new(3);
    assert_eq!(d.period(), 3);
    assert!(!d.is_ready());
    d.update(&hlc(10.0, 8.0, 9.0));
    assert!(!d.is_ready());
    d.update(&hlc(11.0, 9.0, 10.0));
    assert!(!d.is_ready());
    let _ = d.update(&hlc(9.0, 7.0, 8.0));
    assert!(d.is_ready());
}

#[test]
fn middle_is_midpoint_of_upper_and_lower() {
    let mut d = Donchian::new(3);
    for (h, l) in [(10.0, 8.0), (11.0, 9.0), (9.0, 7.0)] {
        d.update(&hlc(h, l, 9.0));
    }
    let v = d.value().unwrap();
    assert_eq!(v.upper, dec!(11));
    assert_eq!(v.lower, dec!(7));
    assert_dec_close(v.middle, dec!(9));
}

#[test]
fn window_slides_over_last_period_bars() {
    let mut d = Donchian::new(2);
    let v = d.update(&hlc(10.0, 8.0, 9.0));
    assert_eq!(v, None);
    let v = d.update(&hlc(11.0, 9.0, 10.0)).unwrap();
    assert_eq!(v.upper, dec!(11));
    assert_eq!(v.lower, dec!(8));
    let v = d.update(&hlc(9.0, 7.0, 8.0)).unwrap();
    assert_eq!(v.upper, dec!(11));
    assert_eq!(v.lower, dec!(7));
    let v = d.update(&hlc(5.0, 2.0, 3.0)).unwrap();
    assert_eq!(v.upper, dec!(9));
    assert_eq!(v.lower, dec!(2));
}

#[test]
fn period_one_echoes_input_bar() {
    let mut d = Donchian::new(1);
    let v = d.update(&hlc(10.0, 8.0, 9.0)).unwrap();
    assert_eq!(v.upper, dec!(10));
    assert_eq!(v.lower, dec!(8));
    assert_dec_close(v.middle, dec!(9));
}

#[test]
fn reset_clears_state() {
    let mut d = Donchian::new(2);
    d.update(&hlc(10.0, 8.0, 9.0));
    d.update(&hlc(11.0, 9.0, 10.0));
    assert!(d.is_ready());
    d.reset();
    assert!(!d.is_ready());
    assert_eq!(d.update(&hlc(5.0, 3.0, 4.0)), None);
}

fn reference(
    bars: &[(Decimal, Decimal)],
    period: usize,
) -> Vec<Option<(Decimal, Decimal, Decimal)>> {
    let mut out = Vec::with_capacity(bars.len());
    for i in 0..bars.len() {
        if i + 1 < period {
            out.push(None);
            continue;
        }
        let window = &bars[i + 1 - period..=i];
        let upper = window.iter().map(|(h, _)| *h).max().unwrap();
        let lower = window.iter().map(|(_, l)| *l).min().unwrap();
        out.push(Some((upper, lower, (upper + lower) / Decimal::TWO)));
    }
    out
}

#[test]
fn oracle_against_o_n_reference() {
    let seq: Vec<(Decimal, Decimal)> = (0..500)
        .map(|i: usize| {
            let a = Decimal::new(
                ((i.wrapping_mul(1664525).wrapping_add(1013904223)) % 10000) as i64,
                2,
            );
            let b = Decimal::new(
                ((i.wrapping_mul(22695477).wrapping_add(1)) % 5000) as i64,
                2,
            );
            let lo = a.min(b);
            let hi = a.max(b) + dec!(0.01);
            (hi, lo)
        })
        .collect();
    for period in [1, 2, 3, 5, 20] {
        let mut d = Donchian::new(period);
        let expected = reference(&seq, period);
        for (i, (h, l)) in seq.iter().enumerate() {
            let got = d.update(&hlc_dec(*h, *l, *l));
            match expected[i] {
                None => assert_eq!(got, None, "period {period} index {i}"),
                Some((eu, el, em)) => {
                    let gv = got.unwrap();
                    assert_dec_close(gv.upper, eu);
                    assert_dec_close(gv.lower, el);
                    assert_dec_close(gv.middle, em);
                }
            }
        }
    }
}
