//! Unit & conformance tests for `crate::adr_pct`.

mod common;
use common::hlc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{AdrPct, Indicator};

fn batch_adr_pct(bars: &[(Decimal, Decimal)], period: usize) -> Vec<Option<Decimal>> {
    let n = Decimal::from(period);
    let mut out = Vec::with_capacity(bars.len());
    for i in 0..bars.len() {
        if i + 1 < period {
            out.push(None);
            continue;
        }
        let sum: Decimal = bars[i + 1 - period..=i].iter().map(|(h, l)| *h / *l).sum();
        out.push(Some((sum / n - Decimal::ONE) * Decimal::ONE_HUNDRED));
    }
    out
}

#[test]
#[should_panic(expected = "ADR% period must be positive")]
fn zero_period_panics() {
    let _ = AdrPct::new(0);
}

#[test]
fn returns_none_until_warmed_up() {
    let mut adr = AdrPct::new(3);
    assert_eq!(adr.period(), 3);
    assert!(!adr.is_ready());
    assert_eq!(adr.update(&hlc(105.0, 100.0, 102.0)), None);
    assert!(!adr.is_ready());
    assert_eq!(adr.update(&hlc(110.0, 100.0, 105.0)), None);
    assert!(!adr.is_ready());
    // Mean of (1.05, 1.10, 1.15) = 1.10 -> (1.10 - 1) * 100 = 10
    let v = adr.update(&hlc(115.0, 100.0, 110.0)).unwrap();
    assert!(adr.is_ready());
    assert_eq!(v, dec!(10));
    assert_eq!(adr.value(), Some(dec!(10)));
}

#[test]
fn computes_mean_of_ratios_not_ratio_of_means() {
    // Distinguish sum(H_i/L_i)/N - 1 from mean(H)/mean(L) - 1:
    // Bar 1: H=200, L=100 => H/L = 2.0
    // Bar 2: H=15,  L=10  => H/L = 1.5
    // Mean of ratios = 1.75 => ADR% = 75.
    // (Whereas mean(H)/mean(L) - 1 = 107.5/55 - 1 = 95.45...)
    let mut adr = AdrPct::new(2);
    assert_eq!(adr.update_hl(dec!(200), dec!(100)), None);
    let v = adr.update_hl(dec!(15), dec!(10)).unwrap();
    assert_eq!(v, dec!(75));
}

#[test]
fn golden_vector_qullamaggie_adr20() {
    let mut adr = AdrPct::new(20);
    // 20 alternating bars: 10 with 4% range (H=104, L=100), 10 with 6% range (H=106, L=100)
    // Exact mean ratio = 1.05 -> ADR%(20) = 5.00
    for i in 0..19 {
        let h = if i % 2 == 0 { dec!(104) } else { dec!(106) };
        assert_eq!(adr.update_hl(h, dec!(100)), None);
    }
    let v20 = adr.update_hl(dec!(106), dec!(100)).unwrap();
    assert_eq!(v20, dec!(5));

    // Bar 21 drops bar 0 (104/100 = 1.04) and adds 120/100 = 1.20:
    // delta = (1.20 - 1.04) / 20 * 100 = 0.80 -> 5.80
    let v21 = adr.update_hl(dec!(120), dec!(100)).unwrap();
    assert_eq!(v21, dec!(5.8));
}

#[test]
fn non_positive_low_returns_none_without_mutating_state() {
    let mut adr = AdrPct::new(2);
    assert_eq!(adr.update_hl(dec!(105), dec!(100)), None);
    assert_eq!(adr.update_hl(dec!(105), Decimal::ZERO), None);
    assert_eq!(adr.update_hl(dec!(105), dec!(-5)), None);
    assert_eq!(adr.value(), None);
    // Next valid bar completes the 2-bar window with the first bar.
    assert_eq!(adr.update_hl(dec!(115), dec!(100)), Some(dec!(10)));
}

#[test]
fn reset_clears_state() {
    let mut adr = AdrPct::new(2);
    adr.update_hl(dec!(105), dec!(100));
    adr.update_hl(dec!(115), dec!(100));
    assert!(adr.is_ready());
    adr.reset();
    assert!(!adr.is_ready());
    assert_eq!(adr.value(), None);
    assert_eq!(adr.update_hl(dec!(105), dec!(100)), None);
}

#[test]
fn f64_shim_matches_and_rejects_non_finite_without_mutation() {
    let mut adr = AdrPct::new(2);
    assert_eq!(adr.update_hl_f64(f64::NAN, 100.0), None);
    assert_eq!(adr.update_hl_f64(105.0, 100.0), None);
    assert_eq!(adr.update_hl_f64(115.0, 100.0), Some(10.0));
}

#[test]
fn streaming_matches_batch_reference() {
    let bars: Vec<(Decimal, Decimal)> = (0..60)
        .map(|i| {
            let low = Decimal::from(100 + (i * 7) % 31);
            let high = low + Decimal::from(1 + (i * 13) % 19) / Decimal::from(2);
            (high, low)
        })
        .collect();
    for period in [1, 5, 14, 20] {
        let mut adr = AdrPct::new(period);
        let expected = batch_adr_pct(&bars, period);
        for (i, (h, l)) in bars.iter().enumerate() {
            assert_eq!(
                adr.update_hl(*h, *l),
                expected[i],
                "period {period} row {i}"
            );
        }
    }
}
