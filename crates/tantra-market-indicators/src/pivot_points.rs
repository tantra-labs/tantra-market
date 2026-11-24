//! Pivot Points (Classic, Fibonacci, Woodie).

use rust_decimal::Decimal;

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

/// The support/resistance levels derived from one bar, computed in `Decimal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PivotValue {
    /// The pivot level.
    pub pivot: Decimal,
    /// Resistance 1.
    pub r1: Decimal,
    /// Support 1.
    pub s1: Decimal,
    /// Resistance 2.
    pub r2: Decimal,
    /// Support 2.
    pub s2: Decimal,
    /// Resistance 3 (`None` for Woodie).
    pub r3: Option<Decimal>,
    /// Support 3 (`None` for Woodie).
    pub s3: Option<Decimal>,
}

/// Daily/period pivot levels computed from each bar's high, low and close in `Decimal`.
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

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`.
    pub fn update_hlc(
        &mut self,
        high: Decimal,
        low: Decimal,
        close: Decimal,
    ) -> Option<PivotValue> {
        let range = high - low;
        let two = Decimal::TWO;
        let three = Decimal::from(3);
        let four = Decimal::from(4);
        let fib_382 = Decimal::new(382, 3);
        let fib_618 = Decimal::new(618, 3);

        let v = match self.style {
            PivotStyle::Classic => {
                let pivot = (high + low + close) / three;
                PivotValue {
                    pivot,
                    r1: two * pivot - low,
                    s1: two * pivot - high,
                    r2: pivot + range,
                    s2: pivot - range,
                    r3: Some(high + two * (pivot - low)),
                    s3: Some(low - two * (high - pivot)),
                }
            }
            PivotStyle::Fibonacci => {
                let pivot = (high + low + close) / three;
                PivotValue {
                    pivot,
                    r1: pivot + fib_382 * range,
                    s1: pivot - fib_382 * range,
                    r2: pivot + fib_618 * range,
                    s2: pivot - fib_618 * range,
                    r3: Some(pivot + range),
                    s3: Some(pivot - range),
                }
            }
            PivotStyle::Woodie => {
                let pivot = (high + low + two * close) / four;
                PivotValue {
                    pivot,
                    r1: two * pivot - low,
                    s1: two * pivot - high,
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
    type Input<'a> = &'a tantra_core::Bar;
    type Output = PivotValue;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<PivotValue> {
        self.update_hlc(bar.high.0, bar.low.0, bar.close.0)
    }

    fn value(&self) -> Option<PivotValue> {
        self.last
    }

    fn reset(&mut self) {
        self.last = None;
    }
}
