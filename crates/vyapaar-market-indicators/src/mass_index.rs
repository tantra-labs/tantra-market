//! Mass Index (Donald Dorsey).

use std::collections::VecDeque;

use crate::ema::Ema;
use crate::indicator::Indicator;

/// Mass Index.
///
/// `Σᵢ ratio[i]` over the last `period` ratios, where
/// `ratio = EMA(9, range) / EMA(9, EMA(9, range))` and `range = high − low`.
/// The first value appears after `period` ratios have accumulated (each ratio
/// needs its own 9- and 18-bar EMA warmup). A value above `27` is the classic
/// "reversal bulge" signal.
#[derive(Clone, Debug)]
pub struct MassIndex {
    period: usize,
    ema_range: Ema,
    ema_ema_range: Ema,
    ratios: VecDeque<f64>,
    last: Option<f64>,
}

impl MassIndex {
    /// Creates a Mass Index over `period` ratios.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "Mass Index period must be positive");
        Self {
            period,
            ema_range: Ema::new(9),
            ema_ema_range: Ema::new(9),
            ratios: VecDeque::with_capacity(period),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Feeds one bar given as raw `high`/`low` and returns the Mass Index once
    /// warmed up. Identical to [`Indicator::update`] on a bar with those
    /// fields; the close is unused.
    pub fn update_hl(&mut self, high: f64, low: f64) -> Option<f64> {
        let range = (high - low).max(0.0);
        let e1 = self.ema_range.update(range)?;
        let e2 = self.ema_ema_range.update(e1)?;
        let ratio = if e2 == 0.0 { 0.0 } else { e1 / e2 };

        self.ratios.push_back(ratio);
        if self.ratios.len() > self.period {
            self.ratios.pop_front();
        }
        if self.ratios.len() < self.period {
            return None;
        }
        let v = self.ratios.iter().sum();
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for MassIndex {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = f64;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<f64> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        self.update_hl(high, low)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.ema_range.reset();
        self.ema_ema_range.reset();
        self.ratios.clear();
        self.last = None;
    }
}
