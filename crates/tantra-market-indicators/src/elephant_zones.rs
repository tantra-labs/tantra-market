//! Elephant Zones (EE+ session level map bands).
//!
//! Two support / two resistance bands per session, either from historical
//! session-range percentiles (`Percentile` mode) or from a 1-minute opening
//! candle range extension (`OpeningRangeExtension` mode). Sessions are keyed by
//! the Asia/Kolkata calendar date of each bar; the session-fixed contract
//! resets all intraday accumulation at each new session while the percentile
//! rolling window persists across sessions.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tantra_core::Bar;

use crate::indicator::Indicator;

/// The two support / two resistance bands projected by [`ElephantZones`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElephantZonesValue {
    /// Inner support band (nearer the session open). Uses the 25th percentile
    /// of historical `(open - low)` distances.
    pub s1: Decimal,
    /// Outer support band (further below the session open). Uses the 35th
    /// percentile of historical `(open - low)` distances, so `s2 <= s1`.
    pub s2: Decimal,
    /// Inner resistance band (nearer the session open). Uses the 63rd
    /// percentile of historical `(high - open)` distances.
    pub r1: Decimal,
    /// Outer resistance band (further above the session open). Uses the 74th
    /// percentile of historical `(high - open)` distances, so `r1 <= r2`.
    pub r2: Decimal,
}

/// How [`ElephantZones`] projects its bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElephantMode {
    /// Historical session-range percentile bands: every finalized session's
    /// `(open - low)` and `(high - open)` distances are exact-percentile-ranked
    /// over a rolling window, and the bands are anchored at the live session
    /// open. Emits only once the rolling window is full.
    Percentile {
        /// Number of trailing finalized sessions in the rolling window
        /// (default 20 in the EE+ strategy; clamped to `>= 1`).
        rolling_sessions: u32,
    },
    /// Opening-range extension bands: the first bar of each session is the
    /// opening candle; the extension is `first_high - first_low` and the bands
    /// are `anchor +/- 1.618x / 2.618x` of it, anchored at the first-candle
    /// close. Emits from the second bar of the session onward.
    OpeningRangeExtension,
}

/// One finalized session's open / high / low (percentile-mode window entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionRange {
    open: Decimal,
    high: Decimal,
    low: Decimal,
}

/// Session-fixed Elephant Zones indicator (pure, zero floats, wasm-portable).
///
/// ```
/// use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
/// use tantra_market_indicators::{ElephantZones, Indicator};
/// use rust_decimal::Decimal;
///
/// fn bar(open: f64, high: f64, low: f64, close: f64) -> Bar {
///     Bar {
///         instrument: InstrumentId::new(Exchange::Nse, "X"),
///         open: Price::new(Decimal::from_f64_retain(open).unwrap()),
///         high: Price::new(Decimal::from_f64_retain(high).unwrap()),
///         low: Price::new(Decimal::from_f64_retain(low).unwrap()),
///         close: Price::new(Decimal::from_f64_retain(close).unwrap()),
///         volume: Quantity::from_u64(1),
///         timestamp: chrono::Utc::now(),
///     }
/// }
///
/// let mut zones = ElephantZones::opening_range_extension();
/// assert_eq!(zones.update(&bar(100.0, 105.0, 95.0, 102.0)), None); // first bar
/// let v = zones.update(&bar(100.0, 106.0, 96.0, 103.0)).unwrap();
/// assert_eq!(v.s1, Decimal::from_str_exact("85.82").unwrap());
/// ```
#[derive(Debug, Clone)]
pub struct ElephantZones {
    mode: ElephantMode,
    rolling_sessions: usize,
    /// Asia/Kolkata date of the session currently being accumulated.
    session_date: Option<NaiveDate>,
    /// Live session anchor (first-bar open in `Percentile` mode, first-candle
    /// close in `OpeningRangeExtension` mode).
    session_open: Option<Decimal>,
    /// Live session high / low (percentile accumulation).
    session_high: Decimal,
    session_low: Decimal,
    /// Opening-candle range extension (ORE mode).
    ore_ext: Option<Decimal>,
    value: Option<ElephantZonesValue>,
    /// Rolling window of finalized sessions (percentile mode).
    window: Vec<SessionRange>,
    /// `high - low` of every finalized session, kept in lockstep with `window`.
    ranges: Vec<Decimal>,
}

impl ElephantZones {
    /// Creates zones in the given [`ElephantMode`].
    pub fn new(mode: ElephantMode) -> Self {
        let rolling_sessions = match mode {
            ElephantMode::Percentile { rolling_sessions } => rolling_sessions.max(1) as usize,
            ElephantMode::OpeningRangeExtension => 0,
        };
        Self {
            mode,
            rolling_sessions,
            session_date: None,
            session_open: None,
            session_high: Decimal::ZERO,
            session_low: Decimal::ZERO,
            ore_ext: None,
            value: None,
            window: Vec::new(),
            ranges: Vec::new(),
        }
    }

    /// Percentile-mode zones over `rolling_sessions` trailing sessions.
    pub fn percentile(rolling_sessions: u32) -> Self {
        Self::new(ElephantMode::Percentile { rolling_sessions })
    }

    /// Opening-range-extension zones.
    pub fn opening_range_extension() -> Self {
        Self::new(ElephantMode::OpeningRangeExtension)
    }

    /// The configured mode.
    pub fn mode(&self) -> ElephantMode {
        self.mode
    }

    /// The finalized session ranges (`high - low`) in the rolling window,
    /// oldest first. Empty in [`ElephantMode::OpeningRangeExtension`] mode.
    pub fn session_ranges(&self) -> &[Decimal] {
        &self.ranges
    }

    fn update_percentile(&mut self, bar: &Bar, date: NaiveDate) -> Option<ElephantZonesValue> {
        if self.session_date != Some(date) {
            // A new session starts: finalize the previous session's range into
            // the rolling window (kept at most `rolling_sessions` entries).
            if let Some(open) = self.session_open {
                let finished = SessionRange {
                    open,
                    high: self.session_high,
                    low: self.session_low,
                };
                self.window.push(finished);
                self.ranges.push(self.session_high - self.session_low);
                if self.window.len() > self.rolling_sessions {
                    self.window.remove(0);
                    self.ranges.remove(0);
                }
            }
            self.session_date = Some(date);
            self.session_open = Some(bar.open.0);
            self.session_high = bar.high.0;
            self.session_low = bar.low.0;
        } else {
            self.session_high = self.session_high.max(bar.high.0);
            self.session_low = self.session_low.min(bar.low.0);
        }

        // Output only once enough sessions are primed.
        if self.window.len() < self.rolling_sessions {
            return None;
        }
        let open = self.session_open?;
        let mut support_dists: Vec<Decimal> = self.window.iter().map(|s| s.open - s.low).collect();
        let mut resistance_dists: Vec<Decimal> =
            self.window.iter().map(|s| s.high - s.open).collect();
        support_dists.sort();
        resistance_dists.sort();
        let value = ElephantZonesValue {
            s1: open - percentile(&support_dists, 25),
            s2: open - percentile(&support_dists, 35),
            r1: open + percentile(&resistance_dists, 63),
            r2: open + percentile(&resistance_dists, 74),
        };
        self.value = Some(value);
        Some(value)
    }

    fn update_ore(&mut self, bar: &Bar, date: NaiveDate) -> Option<ElephantZonesValue> {
        if self.session_date != Some(date) {
            // New session: the first bar is the opening candle.
            self.session_date = Some(date);
            self.ore_ext = Some(bar.high.0 - bar.low.0);
            self.session_open = Some(bar.close.0); // anchor = first-candle close
            return None;
        }
        // Emit from the second bar of the session onward.
        let anchor = self.session_open?;
        let ext = self.ore_ext?;
        let value = ElephantZonesValue {
            s1: anchor - PHI * ext,
            s2: anchor - PHI_SQUARED * ext,
            r1: anchor + PHI * ext,
            r2: anchor + PHI_SQUARED * ext,
        };
        self.value = Some(value);
        Some(value)
    }
}

/// The session key of a bar: its timestamp converted to Asia/Kolkata and
/// reduced to a calendar date (09:15 IST session date).
fn session_date(timestamp: DateTime<Utc>) -> NaiveDate {
    timestamp
        .with_timezone(&chrono_tz::Asia::Kolkata)
        .date_naive()
}

/// Exact nearest-rank percentile of a sorted, non-empty slice: the `pct`-th
/// percentile is `sorted[ceil(pct/100 * n) - 1]` (no interpolation).
fn percentile(sorted: &[Decimal], pct: u32) -> Decimal {
    let n = sorted.len();
    let pct = pct.min(100);
    let rank = (pct as usize * n).div_ceil(100);
    let rank = rank.clamp(1, n);
    sorted[rank - 1]
}

/// `1.618` as an exact `Decimal` (1618/1000).
const PHI: Decimal = Decimal::from_parts(1618, 0, 0, false, 3);
/// `2.618` as an exact `Decimal` (2618/1000).
const PHI_SQUARED: Decimal = Decimal::from_parts(2618, 0, 0, false, 3);

impl Indicator for ElephantZones {
    type Input<'a> = &'a Bar;
    type Output = ElephantZonesValue;

    fn update(&mut self, bar: &Bar) -> Option<ElephantZonesValue> {
        let date = session_date(bar.timestamp);
        let value = match self.mode {
            ElephantMode::Percentile { .. } => self.update_percentile(bar, date),
            ElephantMode::OpeningRangeExtension => self.update_ore(bar, date),
        };
        if value.is_none() {
            self.value = None;
        }
        value
    }

    fn value(&self) -> Option<ElephantZonesValue> {
        self.value
    }

    fn reset(&mut self) {
        self.session_date = None;
        self.session_open = None;
        self.session_high = Decimal::ZERO;
        self.session_low = Decimal::ZERO;
        self.ore_ext = None;
        self.value = None;
        self.window.clear();
        self.ranges.clear();
    }
}
