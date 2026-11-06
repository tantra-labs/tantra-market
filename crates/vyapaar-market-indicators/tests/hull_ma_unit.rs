//! Unit tests for `crate::hull_ma`.

mod common;
use common::assert_close;
use vyapaar_market_indicators::{HullMa, Indicator};

#[test]
#[should_panic(expected = "Hull MA period must be positive")]
fn zero_period_panics() {
    let _ = HullMa::new(0);
}

#[test]
fn warmup_then_tracks_a_straight_line() {
    // HMA of an arithmetic sequence equals the latest value; period 4 warms
    // up after 4 + 2 − 1 = 5 inputs (sqrt(4) = 2).
    let mut h = HullMa::new(4);
    assert_eq!(h.period(), 4);
    let mut outputs = Vec::new();
    for (i, price) in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0].iter().enumerate() {
        let v = h.update(*price);
        outputs.push(v);
        if i < 4 {
            assert_eq!(v, None);
        }
    }
    assert_close(outputs[4].unwrap(), 5.0);
    assert_close(outputs[5].unwrap(), 6.0);
}

#[test]
fn constant_price_stays_constant() {
    let mut h = HullMa::new(3);
    let mut last = None;
    for _ in 0..20 {
        last = h.update(42.0);
    }
    assert_close(last.unwrap(), 42.0);
}

#[test]
fn reset_requires_a_fresh_window() {
    let mut h = HullMa::new(3);
    for i in 1..=10 {
        h.update(i as f64);
    }
    h.reset();
    assert_eq!(h.value(), None);
    // 3 inputs fill the price window; the 4th produces the first HMA.
    for _ in 0..3 {
        assert_eq!(h.update(1.0), None);
    }
    assert_close(h.update(1.0).unwrap(), 1.0);
}
