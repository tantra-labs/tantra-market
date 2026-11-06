//! Commodity Channel Index (CCI).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// Commodity Channel Index (Lambert).
///
/// `CCI = (TP − SMA(TP, n)) / (0.015 * mean_deviation)`, where
/// `TP = (H + L + C) / 3` and the mean deviation is the average of
/// `|TP − SMA|` over the same window. The first value appears after `period`
/// bars. Extremes beyond ±100 (or ±200) are the classic overbought/oversold
/// signals.
#[derive(Clone, Debug)]
pub struct Cci {
    period: usize,
    tps: VecDeque<f64>,
    last: Option<f64>,
}

impl Cci {
    /// Creates a CCI over `period` bars.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "CCI period must be positive");
        Self {
            period,
            tps: VecDeque::with_capacity(period),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Feeds one bar as raw `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        let tp = (high + low + close) / 3.0;
        self.tps.push_back(tp);
        if self.tps.len() > self.period {
            self.tps.pop_front();
        }
        if self.tps.len() < self.period {
            return None;
        }

        let n = self.period as f64;
        let sma = self.tps.iter().sum::<f64>() / n;
        let md = self.tps.iter().map(|t| (t - sma).abs()).sum::<f64>() / n;
        let v = if md == 0.0 {
            0.0
        } else {
            (tp - sma) / (0.015 * md)
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for Cci {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = f64;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<f64> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        let close = crate::bar_f64(&bar.close);
        self.update_hlc(high, low, close)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.tps.clear();
        self.last = None;
    }
}
