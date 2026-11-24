//! Unit tests for `crate::stochastic`.

mod common;
use common::{assert_dec_close, hlc, hlc_dec};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{Indicator, Stochastic};

#[test]
#[should_panic(expected = "Stochastic period must be positive")]
fn zero_period_panics() {
    let _ = Stochastic::new(0, 2);
}

#[test]
#[should_panic(expected = "Stochastic d_period must be positive")]
fn zero_d_period_panics() {
    let _ = Stochastic::new(3, 0);
}

#[test]
fn reports_periods_and_readiness() {
    let mut s = Stochastic::new(3, 2);
    assert_eq!(s.period(), 3);
    assert_eq!(s.d_period(), 2);
    assert!(!s.is_ready());
    s.update(&hlc(10.0, 5.0, 7.0));
    s.update(&hlc(11.0, 6.0, 9.0));
    assert!(!s.is_ready());
    let _ = s.update(&hlc(9.0, 4.0, 6.0));
    assert!(s.is_ready());
}

#[test]
fn flat_window_gives_fifty() {
    let mut s = Stochastic::new(2, 2);
    s.update(&hlc(10.0, 10.0, 10.0));
    let v = s.update(&hlc(10.0, 10.0, 10.0)).unwrap();
    assert_dec_close(v.k, dec!(50));
}

#[test]
fn known_triple_and_d() {
    let mut s = Stochastic::new(3, 2);
    s.update(&hlc(10.0, 5.0, 7.0));
    s.update(&hlc(11.0, 6.0, 9.0));
    let v = s.update(&hlc(9.0, 4.0, 6.0)).unwrap();
    let expected_k = dec!(100) * dec!(2) / dec!(7);
    assert_dec_close(v.k, expected_k);
    assert_eq!(v.d, None);
    let v2 = s.update(&hlc(12.0, 7.0, 10.0)).unwrap();
    assert_dec_close(v2.k, dec!(75));
    assert_dec_close(v2.d.unwrap(), (expected_k + dec!(75)) / Decimal::TWO);
}

#[test]
fn reset_clears_state() {
    let mut s = Stochastic::new(2, 2);
    s.update(&hlc(10.0, 5.0, 7.0));
    s.update(&hlc(11.0, 6.0, 9.0));
    assert!(s.is_ready());
    s.reset();
    assert!(!s.is_ready());
    assert_eq!(s.update(&hlc(10.0, 5.0, 7.0)), None);
}

fn reference(
    bars: &[(Decimal, Decimal, Decimal)],
    period: usize,
    d_period: usize,
) -> Vec<Option<(Decimal, Option<Decimal>)>> {
    let mut out = Vec::with_capacity(bars.len());
    let mut ks: Vec<Decimal> = Vec::new();
    for i in 0..bars.len() {
        if i + 1 < period {
            out.push(None);
            continue;
        }
        let window = &bars[i + 1 - period..=i];
        let hh = window.iter().map(|(h, _, _)| *h).max().unwrap();
        let ll = window.iter().map(|(_, l, _)| *l).min().unwrap();
        let close = bars[i].2;
        let denom = hh - ll;
        let k = if denom.is_zero() {
            dec!(50)
        } else {
            dec!(100) * (close - ll) / denom
        };
        ks.push(k);
        let d = if ks.len() >= d_period {
            let sum: Decimal = ks[ks.len() - d_period..].iter().copied().sum();
            Some(sum / Decimal::from(d_period))
        } else {
            None
        };
        out.push(Some((k, d)));
    }
    out
}

#[test]
fn oracle_against_o_n_reference() {
    let seq: Vec<(Decimal, Decimal, Decimal)> = (0..500)
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
            let cl = lo + (hi - lo) * dec!(0.3);
            (hi, lo, cl)
        })
        .collect();
    for period in [1, 2, 3, 5, 20] {
        for d_period in [1, 2, 5] {
            let mut s = Stochastic::new(period, d_period);
            let expected = reference(&seq, period, d_period);
            for (i, (h, l, c)) in seq.iter().enumerate() {
                let got = s.update(&hlc_dec(*h, *l, *c));
                match expected[i] {
                    None => assert_eq!(got, None, "period {period},{d_period} index {i}"),
                    Some((ek, ed)) => {
                        let gv = got.unwrap();
                        assert_dec_close(gv.k, ek);
                        match (gv.d, ed) {
                            (None, None) => {}
                            (Some(a), Some(b)) => assert_dec_close(a, b),
                            _ => panic!("d mismatch at {i}"),
                        }
                    }
                }
            }
        }
    }
}
