//! Unit tests for `crate::hull_ma`.

mod common;
use common::assert_dec_close;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_indicators::{HullMa, Indicator};

#[test]
#[should_panic(expected = "Hull MA period must be positive")]
fn zero_period_panics() {
    let _ = HullMa::new(0);
}

#[test]
fn warmup_then_tracks_a_straight_line() {
    // HMA of an arithmetic sequence equals the latest value; period 4 warms
    // up after 4 + 2 - 1 = 5 inputs (sqrt(4) = 2).
    let mut h = HullMa::new(4);
    assert_eq!(h.period(), 4);
    let mut outputs = Vec::new();
    for (i, price) in [dec!(1), dec!(2), dec!(3), dec!(4), dec!(5), dec!(6)]
        .iter()
        .enumerate()
    {
        let v = h.update(*price);
        outputs.push(v);
        if i < 4 {
            assert_eq!(v, None);
        }
    }
    assert_dec_close(outputs[4].unwrap(), dec!(5));
    assert_dec_close(outputs[5].unwrap(), dec!(6));
}

#[test]
fn constant_price_stays_constant() {
    let mut h = HullMa::new(3);
    let mut last = None;
    for _ in 0..20 {
        last = h.update(dec!(42));
    }
    assert_dec_close(last.unwrap(), dec!(42));
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut h = HullMa::new(3);
    for i in 1..=10 {
        h.update(Decimal::from(i));
    }
    h.reset();
    assert_eq!(h.value(), None);
    // 3 inputs fill the price window; the 4th produces the first HMA.
    for _ in 0..3 {
        assert_eq!(h.update(dec!(1)), None);
    }
    assert_dec_close(h.update(dec!(1)).unwrap(), dec!(1));
}
