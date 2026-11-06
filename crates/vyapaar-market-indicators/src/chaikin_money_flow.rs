//! Chaikin Money Flow (CMF).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// Chaikin Money Flow.
///
/// `CMF = Σ money_flow_volume / Σ volume` over the last `period` bars, where
/// `money_flow_volume = money_flow_multiplier * volume`. The range is `[−1, 1]`;
/// a persistently positive CMF confirms accumulation.
#[derive(Clone, Debug)]
pub struct ChaikinMoneyFlow {
    period: usize,
    flows: VecDeque<f64>,
    volumes: VecDeque<f64>,
    last: Option<f64>,
}

impl ChaikinMoneyFlow {
    /// Creates a CMF over `period` bars.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "CMF period must be positive");
        Self {
            period,
            flows: VecDeque::with_capacity(period),
            volumes: VecDeque::with_capacity(period),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Feeds one bar as raw `high`/`low`/`close`/`volume`.
    pub fn update_hlcv(&mut self, high: f64, low: f64, close: f64, volume: f64) -> Option<f64> {
        let flow = crate::mfi::Mfi::multiplier(high, low, close) * volume;
        self.flows.push_back(flow);
        self.volumes.push_back(volume);
        if self.flows.len() > self.period {
            self.flows.pop_front();
            self.volumes.pop_front();
        }
        if self.flows.len() < self.period {
            return None;
        }
        let sum_flow: f64 = self.flows.iter().sum();
        let sum_vol: f64 = self.volumes.iter().sum();
        let v = if sum_vol == 0.0 {
            0.0
        } else {
            sum_flow / sum_vol
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for ChaikinMoneyFlow {
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
        self.volumes.clear();
        self.last = None;
    }
}
