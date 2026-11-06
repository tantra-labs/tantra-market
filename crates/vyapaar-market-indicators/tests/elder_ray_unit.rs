//! Unit tests for `crate::elder_ray`.

mod common;
use common::{assert_close, hlc};
use vyapaar_market_indicators::{ElderRay, Indicator};

#[test]
#[should_panic(expected = "Elder Ray period must be positive")]
fn zero_period_panics() {
    let _ = ElderRay::new(0);
}

#[test]
fn first_value_appears_once_the_ema_primes() {
    let mut e = ElderRay::new(2);
    assert_eq!(e.period(), 2);
    assert_eq!(e.update(&hlc(12.0, 10.0, 11.0)), None);
    let v = e.update(&hlc(13.0, 11.0, 12.0)).unwrap();
    // EMA(2) seed = (11 + 12) / 2 = 11.5.
    assert_close(v.bull_power, 13.0 - 11.5); // 1.5
    assert_close(v.bear_power, 11.0 - 11.5); // -0.5
}

#[test]
fn rising_trend_shows_positive_bull_power() {
    let mut e = ElderRay::new(3);
    let mut last = None;
    for i in 1..=10 {
        last = e.update_hlc(i as f64 + 1.0, i as f64, i as f64 + 0.5);
    }
    assert!(last.unwrap().bull_power > 0.0);
}

#[test]
fn reset_requires_a_fresh_ema_seed() {
    let mut e = ElderRay::new(2);
    e.update(&hlc(12.0, 10.0, 11.0));
    e.update(&hlc(13.0, 11.0, 12.0));
    e.reset();
    assert_eq!(e.value(), None);
    assert_eq!(e.update(&hlc(12.0, 10.0, 11.0)), None);
}
