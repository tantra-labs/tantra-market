//! Money Flow Index (MFI).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// Money Flow Index.
///
/// `MFI = 100 − 100 / (1 + positive_flow / negative_flow)` over the last
/// `period` bars, where each bar contributes `money_flow_multiplier * volume`
/// and the multiplier is `((C − L) − (H − C)) / (H − L)` (zero for a
/// doji/zero-range bar). Range is `[0, 100]`; above 80 / below 20 are the
/// classic overbought / oversold signals.
#[derive(Clone, Debug)]
pub struct Mfi {
    period: usize,
    flows: VecDeque<f64>,
    last: Option<f64>,
}

impl Mfi {
    /// Creates an MFI over `period` bars.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "MFI period must be positive");
        Self {
            period,
            flows: VecDeque::with_capacity(period),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// The money-flow multiplier of a bar: `((C − L) − (H − C)) / (H − L)`,
    /// zero when the bar has no range.
    pub fn multiplier(high: f64, low: f64, close: f64) -> f64 {
        let range = high - low;
        if range <= 0.0 {
            0.0
        } else {
            ((close - low) - (high - close)) / range
        }
    }

    /// Feeds one bar as raw `high`/`low`/`close`/`volume`.
    pub fn update_hlcv(&mut self, high: f64, low: f64, close: f64, volume: f64) -> Option<f64> {
        let flow = Self::multiplier(high, low, close) * volume;
        self.flows.push_back(flow);
        if self.flows.len() > self.period {
            self.flows.pop_front();
        }
        if self.flows.len() < self.period {
            return None;
        }

        let positive: f64 = self.flows.iter().copied().filter(|f| *f > 0.0).sum();
        let negative: f64 = self
            .flows
            .iter()
            .copied()
            .filter(|f| *f < 0.0)
            .map(|f| -f)
            .sum();
        let v = if negative == 0.0 {
            if positive == 0.0 { 50.0 } else { 100.0 }
        } else if positive == 0.0 {
            0.0
        } else {
            100.0 - 100.0 / (1.0 + positive / negative)
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for Mfi {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = f64;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<f64> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        let close = crate::bar_f64(&bar.close);
        let volume = crate::quantity_f64(&bar.volume);
        self.update_hlcv(high, low, close, volume)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.flows.clear();
        self.last = None;
    }
}
