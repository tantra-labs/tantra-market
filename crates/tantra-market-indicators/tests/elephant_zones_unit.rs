//! Unit tests for [`tantra_market_indicators::ElephantZones`].
//!
//! Sessions are keyed by the Asia/Kolkata calendar date; bars are stamped at
//! 04:00 UTC == 09:30 IST so the IST calendar date equals the day supplied to
//! the helper. All timestamps are explicit — never `Utc::now()`.

use std::str::FromStr;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
use tantra_market_indicators::{ElephantMode, ElephantZones, ElephantZonesValue, Indicator};

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn bar(open: Decimal, high: Decimal, low: Decimal, close: Decimal, day: u32, minute: u32) -> Bar {
    let timestamp = Utc
        .with_ymd_and_hms(2026, 1, day, 4, 0, 0)
        .single()
        .unwrap()
        + chrono::Duration::minutes(minute as i64);
    Bar {
        instrument: InstrumentId::try_new(Exchange::Nse, "X").unwrap(),
        open: Price::try_new(open).unwrap(),
        high: Price::try_new(high).unwrap(),
        low: Price::try_new(low).unwrap(),
        close: Price::try_new(close).unwrap(),
        volume: Quantity::from_u64(1),
        timestamp,
    }
}

fn pct_bar(open: i32, high: i32, low: i32, close: i32, day: u32, minute: u32) -> Bar {
    bar(
        Decimal::from(open),
        Decimal::from(high),
        Decimal::from(low),
        Decimal::from(close),
        day,
        minute,
    )
}

/// Percentile golden: three seeded sessions then a live session; bands are the
/// exact nearest-rank percentiles (25/35 support, 63/74 resistance) anchored
/// at the live session open.
#[test]
fn percentile_golden_bands() {
    let mut zones = ElephantZones::percentile(3);

    // Session 2026-01-05: open 100, high 110, low 95 -> support 5, resistance 10.
    assert_eq!(zones.update(&pct_bar(100, 110, 95, 105, 5, 0)), None);
    assert_eq!(zones.update(&pct_bar(100, 108, 96, 103, 5, 5)), None);
    // Session 2026-01-06: open 100, high 115, low 92 -> support 8, resistance 15.
    assert_eq!(zones.update(&pct_bar(100, 115, 92, 108, 6, 0)), None);
    // Session 2026-01-07: open 100, high 120, low 90 -> support 10, resistance 20.
    assert_eq!(zones.update(&pct_bar(100, 120, 90, 110, 7, 0)), None);
    assert_eq!(zones.update(&pct_bar(100, 121, 91, 112, 7, 5)), None);

    // Session 2026-01-08 now starts; the window holds sessions 5-7 (3 primed).
    // support dists [5, 8, 10]: P25 = 5 (rank 1), P35 = 8 (rank 2).
    // resistance dists [10, 15, 21]: P63 = 15 (rank 2), P74 = 21 (rank 3)
    // (session 7's second bar raises its high to 121).
    let expected = ElephantZonesValue {
        s1: Decimal::from(95),
        s2: Decimal::from(92),
        r1: Decimal::from(115),
        r2: Decimal::from(121),
    };
    let v = zones.update(&pct_bar(100, 118, 98, 112, 8, 0)).unwrap();
    assert_eq!(v, expected);
    // Same session, same anchor/window -> identical bands on the next bar.
    let v2 = zones.update(&pct_bar(100, 119, 97, 113, 8, 5)).unwrap();
    assert_eq!(v2, expected);
    assert_eq!(zones.value(), Some(expected));

    // The rolling session ranges (high - low) of the three finalized sessions.
    assert_eq!(
        zones.session_ranges(),
        &[Decimal::from(15), Decimal::from(23), Decimal::from(31)]
    );
}

/// The rolling window drops the oldest session once it exceeds its cap, and
/// `session_ranges` tracks the live window.
#[test]
fn rolling_window_caps_and_drops_oldest() {
    let mut zones = ElephantZones::percentile(2);
    // Session 5 starts: no window yet.
    assert_eq!(zones.update(&pct_bar(100, 110, 95, 95, 5, 0)), None);
    // Session 6 starts: window = [s5] (1 < 2, still not primed).
    assert_eq!(zones.update(&pct_bar(100, 115, 92, 92, 6, 0)), None);
    // Session 7 starts: window = [s5, s6] is now full -> primed.
    assert!(zones.update(&pct_bar(100, 120, 90, 90, 7, 0)).is_some());
    // Session 8 starts: d7 is finalized and the window caps to [s6, s7].
    assert!(zones.update(&pct_bar(100, 118, 98, 98, 8, 0)).is_some());
    assert_eq!(
        zones.session_ranges(),
        &[Decimal::from(23), Decimal::from(30)]
    );
    // Session 9 starts: window becomes [s7, s8].
    assert!(zones.update(&pct_bar(100, 122, 94, 94, 9, 0)).is_some());
    assert_eq!(
        zones.session_ranges(),
        &[Decimal::from(30), Decimal::from(20)]
    );
}

/// OpeningRangeExtension: extension math is `anchor -/+ 1.618x / 2.618x` of the
/// first-candle range, anchored at the first-candle close, emitted from the
/// second bar of the session onward.
#[test]
fn ore_extension_math() {
    let mut zones = ElephantZones::opening_range_extension();

    // First bar of session: opening candle (100, 105, 95) close 102.
    // ext = 10, anchor = 102.
    assert_eq!(zones.update(&pct_bar(100, 105, 95, 102, 5, 0)), None);

    // Second bar onward: s1 = 102 - 16.18, s2 = 102 - 26.18, r1 = 118.18, r2 = 128.18.
    let expected = ElephantZonesValue {
        s1: dec("85.82"),
        s2: dec("75.82"),
        r1: dec("118.18"),
        r2: dec("128.18"),
    };
    let v = zones.update(&pct_bar(100, 106, 96, 103, 5, 1)).unwrap();
    assert_eq!(v, expected);
    assert_eq!(zones.value(), Some(expected));
    // A later bar of the same session still emits the same bands.
    assert_eq!(
        zones.update(&pct_bar(100, 107, 97, 104, 5, 2)).unwrap(),
        expected
    );

    // New session resets the opening candle: (50, 52, 48) close 51 -> ext 4.
    assert_eq!(zones.update(&pct_bar(50, 52, 48, 51, 6, 0)), None);
    let v2 = zones.update(&pct_bar(50, 53, 49, 52, 6, 1)).unwrap();
    assert_eq!(v2.s1, dec("44.528"));
    assert_eq!(v2.s2, dec("40.528"));
    assert_eq!(v2.r1, dec("57.472"));
    assert_eq!(v2.r2, dec("61.472"));
}

/// `None` before priming: a percentile window of 20 sessions never emits while
/// only one session has been finalized.
#[test]
fn none_before_priming() {
    let mut zones = ElephantZones::percentile(20);
    assert_eq!(zones.value(), None);
    for minute in 0..5 {
        assert_eq!(zones.update(&pct_bar(100, 110, 95, 105, 5, minute)), None);
    }
    // A second session finalizes the first, but 1 < 20 still.
    assert_eq!(zones.update(&pct_bar(100, 112, 94, 106, 6, 0)), None);
    assert_eq!(zones.value(), None);
    // ORE mode emits only from the second bar.
    let mut ore = ElephantZones::opening_range_extension();
    assert_eq!(ore.update(&pct_bar(100, 110, 95, 105, 5, 0)), None);
    assert!(ore.update(&pct_bar(100, 111, 96, 106, 5, 1)).is_some());
}

/// `reset` clears all intraday state and the rolling window.
#[test]
fn reset_clears_state() {
    let mut zones = ElephantZones::percentile(3);
    for (day, o, h, l) in [(5, 100, 110, 95), (6, 100, 115, 92), (7, 100, 120, 90)] {
        assert_eq!(zones.update(&pct_bar(o, h, l, o, day, 0)), None);
    }
    assert!(zones.update(&pct_bar(100, 118, 98, 112, 8, 0)).is_some());
    assert!(!zones.session_ranges().is_empty());

    zones.reset();
    assert_eq!(zones.value(), None);
    assert!(zones.session_ranges().is_empty());
    assert_eq!(zones.update(&pct_bar(100, 110, 95, 105, 9, 0)), None);
}

/// The mode is preserved and reportable.
#[test]
fn mode_roundtrip() {
    let p = ElephantZones::new(ElephantMode::Percentile {
        rolling_sessions: 7,
    });
    assert_eq!(
        p.mode(),
        ElephantMode::Percentile {
            rolling_sessions: 7
        }
    );
    let ore = ElephantZones::new(ElephantMode::OpeningRangeExtension);
    assert_eq!(ore.mode(), ElephantMode::OpeningRangeExtension);
}
