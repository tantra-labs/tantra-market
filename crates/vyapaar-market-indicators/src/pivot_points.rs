//! Pivot Points (Classic, Fibonacci, Woodie).

use crate::indicator::Indicator;

/// The pivot style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PivotStyle {
    /// Classic floor-trader pivots: `P = (H + L + C) / 3`.
    Classic,
    /// Fibonacci retracement pivots off the Classic pivot.
    Fibonacci,
    /// Woodie pivots: `P = (H + L + 2C) / 4` (no R3/S3).
    Woodie,
}

/// The support/resistance levels derived from one bar.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PivotValue {
    /// The pivot level.
    pub pivot: f64,
    /// Resistance 1.
    pub r1: f64,
    /// Support 1.
    pub s1: f64,
    /// Resistance 2.
    pub r2: f64,
    /// Support 2.
    pub s2: f64,
    /// Resistance 3 (`None` for Woodie).
    pub r3: Option<f64>,
    /// Support 3 (`None` for Woodie).
    pub s3: Option<f64>,
}

/// Daily/period pivot levels computed from each bar's high, low and close.
///
/// Every bar yields a value from the first (the levels use that bar's own
/// high/low/close, which is how intraday indicator feeds derive "today's"
/// levels from the running daily bar).
#[derive(Clone, Debug)]
pub struct PivotPoints {
    style: PivotStyle,
    last: Option<PivotValue>,
}

impl PivotPoints {
    /// Creates pivot-point levels for a style.
    pub fn new(style: PivotStyle) -> Self {
        Self { style, last: None }
    }

    /// Returns the configured style.
    pub fn style(&self) -> PivotStyle {
        self.style
    }

    /// Feeds one bar as raw `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: f64, low: f64, close: f64) -> Option<PivotValue> {
        let range = high - low;
        let v = match self.style {
            PivotStyle::Classic => {
                let pivot = (high + low + close) / 3.0;
                PivotValue {
                    pivot,
                    r1: 2.0 * pivot - low,
                    s1: 2.0 * pivot - high,
                    r2: pivot + range,
                    s2: pivot - range,
                    r3: Some(high + 2.0 * (pivot - low)),
                    s3: Some(low - 2.0 * (high - pivot)),
                }
            }
            PivotStyle::Fibonacci => {
                let pivot = (high + low + close) / 3.0;
                PivotValue {
                    pivot,
                    r1: pivot + 0.382 * range,
                    s1: pivot - 0.382 * range,
                    r2: pivot + 0.618 * range,
                    s2: pivot - 0.618 * range,
                    r3: Some(pivot + range),
                    s3: Some(pivot - range),
                }
            }
            PivotStyle::Woodie => {
                let pivot = (high + low + 2.0 * close) / 4.0;
                PivotValue {
                    pivot,
                    r1: 2.0 * pivot - low,
                    s1: 2.0 * pivot - high,
                    r2: pivot + range,
                    s2: pivot - range,
                    r3: None,
                    s3: None,
                }
            }
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for PivotPoints {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = PivotValue;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<PivotValue> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        let close = crate::bar_f64(&bar.close);
        self.update_hlc(high, low, close)
    }

    fn value(&self) -> Option<PivotValue> {
        self.last
    }

    fn reset(&mut self) {
        self.last = None;
    }
}
