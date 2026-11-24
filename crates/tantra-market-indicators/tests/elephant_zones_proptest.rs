//! Proptest invariants for [`tantra_market_indicators::ElephantZones`]:
//! bands always stay ordered (`s2 <= s1` and `r1 <= r2`).

use chrono::{TimeZone, Utc};
use proptest::collection::vec as prop_vec;
use proptest::prelude::*;
use rust_decimal::Decimal;
use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
use tantra_market_indicators::{ElephantZones, Indicator};

/// `(open, high, low, close)` with `low <= open <= high` (the bands are
/// anchored at the open and derived from the open / high / low distances).
fn bar_tuple() -> impl Strategy<Value = (u32, u32, u32, u32)> {
    (0u32..1000u32, 0u32..1000u32, 0u32..1000u32, 0u32..1000u32)
        .prop_filter("low <= open <= high", |&(o, h, l, _)| l <= o && o <= h)
}

/// A bar at an explicit timestamp: session `day` at 04:00 UTC (09:30 IST),
/// `minute` offset intra-session.
fn make_bar(o: u32, h: u32, l: u32, c: u32, day: u32, minute: u32) -> Bar {
    let timestamp = Utc
        .with_ymd_and_hms(2026, 1, day, 4, 0, 0)
        .single()
        .unwrap()
        + chrono::Duration::minutes(minute as i64);
    Bar {
        instrument: InstrumentId::try_new(Exchange::Nse, "X").unwrap(),
        open: Price::try_new(Decimal::from(o)).unwrap(),
        high: Price::try_new(Decimal::from(h)).unwrap(),
        low: Price::try_new(Decimal::from(l)).unwrap(),
        close: Price::try_new(Decimal::from(c)).unwrap(),
        volume: Quantity::from_u64(1),
        timestamp,
    }
}

proptest! {
    /// Percentile mode: with a primed rolling window the two support bands are
    /// ordered `s2 <= s1` and the two resistance bands `r1 <= r2`, and the
    /// supports never cross the resistances (`s1 <= r1`).
    #[test]
    fn percentile_bands_remain_ordered(
        sessions in prop_vec(prop_vec(bar_tuple(), 1..4), 4..10),
    ) {
        let mut zones = ElephantZones::percentile(3);
        for (si, session) in sessions.iter().enumerate() {
            for (bi, (o, h, l, c)) in session.iter().enumerate() {
                let bar = make_bar(*o, *h, *l, *c, si as u32 + 1, bi as u32);
                if let Some(v) = zones.update(&bar) {
                    prop_assert!(v.s2 <= v.s1, "s2 must not exceed s1: {v:?}");
                    prop_assert!(v.r1 <= v.r2, "r1 must not exceed r2: {v:?}");
                    prop_assert!(v.s1 <= v.r1, "supports must not cross resistances: {v:?}");
                }
            }
        }
    }

    /// Opening-range-extension mode: the same band ordering holds from the
    /// second bar of every session onward.
    #[test]
    fn ore_bands_remain_ordered(
        sessions in prop_vec(prop_vec(bar_tuple(), 1..4), 1..6),
    ) {
        let mut zones = ElephantZones::opening_range_extension();
        for (si, session) in sessions.iter().enumerate() {
            for (bi, (o, h, l, c)) in session.iter().enumerate() {
                let bar = make_bar(*o, *h, *l, *c, si as u32 + 1, bi as u32);
                if let Some(v) = zones.update(&bar) {
                    prop_assert!(v.s2 <= v.s1, "s2 must not exceed s1: {v:?}");
                    prop_assert!(v.r1 <= v.r2, "r1 must not exceed r2: {v:?}");
                    prop_assert!(v.s1 <= v.r1, "supports must not cross resistances: {v:?}");
                }
            }
        }
    }
}
