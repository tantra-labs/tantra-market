//! Hull Moving Average (HMA).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// Weighted moving average of a slice: weights `n, n-1, …, 1` on the most
/// recent values.
fn wma(values: &[f64]) -> f64 {
    let mut weight = 0.0;
    let mut sum = 0.0;
    for (i, v) in values.iter().enumerate() {
        let w = i as f64 + 1.0;
        weight += w;
        sum += v * w;
    }
    sum / weight
}

/// Hull Moving Average.
///
/// `HMA = WMA(2 * WMA(period/2) − WMA(period), sqrt(period))`, with both
/// inner WMAs over the same (recent) price window. The first value appears
/// after `period + sqrt(period) − 1` inputs.
#[derive(Clone, Debug)]
pub struct HullMa {
    period: usize,
    half: usize,
    sqrt: usize,
    prices: VecDeque<f64>,
    intermediates: VecDeque<f64>,
    last: Option<f64>,
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
        let sqrt = (period as f64).sqrt().round() as usize;
        let sqrt = sqrt.max(1);
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
}

impl Indicator for HullMa {
    type Input<'a> = f64;
    type Output = f64;

    fn update(&mut self, price: f64) -> Option<f64> {
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
        let recent = self.prices.iter().skip(start).copied().collect::<Vec<_>>();
        let intermediate =
            2.0 * wma(&recent) - wma(&self.prices.iter().copied().collect::<Vec<_>>());
        self.intermediates.push_back(intermediate);
        if self.intermediates.len() > self.sqrt {
            self.intermediates.pop_front();
        }
        if self.intermediates.len() < self.sqrt {
            return None;
        }

        let v = wma(&self.intermediates.iter().copied().collect::<Vec<_>>());
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.prices.clear();
        self.intermediates.clear();
        self.last = None;
    }
}
