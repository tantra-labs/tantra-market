//! Chaikin Money Flow (CMF).

use std::collections::VecDeque;

use rust_decimal::Decimal;

use crate::indicator::Indicator;

/// Chaikin Money Flow, computed in `Decimal`.
///
/// `CMF = Σ money_flow_volume / Σ volume` over the last `period` bars, where
/// `money_flow_volume = money_flow_multiplier * volume`. The range is `[−1, 1]`;
/// a persistently positive CMF confirms accumulation.
#[derive(Clone, Debug)]
pub struct ChaikinMoneyFlow {
    period: usize,
    flows: VecDeque<Decimal>,
    volumes: VecDeque<Decimal>,
    last: Option<Decimal>,
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

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`/`volume`.
    pub fn update_hlcv(
        &mut self,
        high: Decimal,
        low: Decimal,
        close: Decimal,
        volume: Decimal,
    ) -> Option<Decimal> {
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
        let sum_flow: Decimal = self.flows.iter().copied().sum();
        let sum_vol: Decimal = self.volumes.iter().copied().sum();
        let v = if sum_vol.is_zero() {
            Decimal::ZERO
        } else {
            sum_flow / sum_vol
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for ChaikinMoneyFlow {
    type Input<'a> = &'a tantra_core::Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<Decimal> {
        self.update_hlcv(bar.high.0, bar.low.0, bar.close.0, bar.volume.0)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.flows.clear();
        self.volumes.clear();
        self.last = None;
    }
}
