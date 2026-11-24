//! Hull Moving Average (HMA).

use std::collections::VecDeque;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, MathematicalOps};

use crate::indicator::Indicator;

/// Weighted moving average of a `VecDeque<Decimal>`: weights `1, 2, …, len`
/// ordered oldest-first so the largest weight falls on the most recent value.
fn wma<'a>(values: impl Iterator<Item = &'a Decimal>) -> Decimal {
    let mut weight = Decimal::ZERO;
    let mut sum = Decimal::ZERO;
    for (i, &v) in values.enumerate() {
        let w = Decimal::from(i + 1);
        weight += w;
        sum += v * w;
    }
    sum / weight
}

/// Hull Moving Average, computed in `Decimal`.
///
/// `HMA = WMA(2 * WMA(period/2) − WMA(period), sqrt(period))`, with both
/// inner WMAs over the same (recent) price window. The first value appears
/// after `period + sqrt(period) − 1` inputs.
#[derive(Clone, Debug)]
pub struct HullMa {
    period: usize,
    half: usize,
    sqrt: usize,
    prices: VecDeque<Decimal>,
    intermediates: VecDeque<Decimal>,
    last: Option<Decimal>,
}

impl HullMa {
    /// Creates a Hull MA over `period` samples.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "Hull MA period must be positive");
        let half = (period / 2).max(1);
        let sqrt = Decimal::from(period)
            .sqrt()
            .and_then(|d| d.round().to_usize())
            .unwrap_or(1)
            .max(1);
        Self {
            period,
            half,
            sqrt,
            prices: VecDeque::with_capacity(period),
            intermediates: VecDeque::with_capacity(sqrt),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// `f64` presentation shim over [`Indicator::update`].
    ///
    /// For chart presentation surfaces only; financial math must use the `Decimal` input.
    /// Non-finite inputs return `None` without mutating state.
    pub fn update_f64(&mut self, price: f64) -> Option<f64> {
        let p = Decimal::try_from(price).ok()?;
        self.update(p).and_then(|v| v.to_f64())
    }
}

impl Indicator for HullMa {
    type Input<'a> = Decimal;
    type Output = Decimal;

    fn update(&mut self, price: Decimal) -> Option<Decimal> {
        self.prices.push_back(price);
        if self.prices.len() > self.period {
            self.prices.pop_front();
        }
        if self.prices.len() < self.period {
            return None;
        }

        // Both WMAs are over the same window; take WMA(period/2) of the most
        // recent `half` values in chronological (oldest-first) order so the
        // weighted-average weights fall on the most recent values.
        let start = self.prices.len() - self.half;
        let intermediate =
            Decimal::TWO * wma(self.prices.iter().skip(start)) - wma(self.prices.iter());
        self.intermediates.push_back(intermediate);
        if self.intermediates.len() > self.sqrt {
            self.intermediates.pop_front();
        }
        if self.intermediates.len() < self.sqrt {
            return None;
        }

        let v = wma(self.intermediates.iter());
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.prices.clear();
        self.intermediates.clear();
        self.last = None;
    }
}
