//! Money Flow Index (MFI).

use std::collections::VecDeque;

use rust_decimal::Decimal;

use crate::indicator::Indicator;

/// Money Flow Index, computed in `Decimal`.
///
/// `MFI = 100 − 100 / (1 + positive_flow / negative_flow)` over the last
/// `period` bars, where each bar contributes `money_flow_multiplier * volume`
/// and the multiplier is `((C − L) − (H − C)) / (H − L)` (zero for a
/// doji/zero-range bar). Range is `[0, 100]`; above 80 / below 20 are the
/// classic overbought / oversold signals.
#[derive(Clone, Debug)]
pub struct Mfi {
    period: usize,
    flows: VecDeque<Decimal>,
    last: Option<Decimal>,
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
    pub fn multiplier(high: Decimal, low: Decimal, close: Decimal) -> Decimal {
        let range = high - low;
        if range <= Decimal::ZERO {
            Decimal::ZERO
        } else {
            ((close - low) - (high - close)) / range
        }
    }

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`/`volume`.
    pub fn update_hlcv(
        &mut self,
        high: Decimal,
        low: Decimal,
        close: Decimal,
        volume: Decimal,
    ) -> Option<Decimal> {
        let flow = Self::multiplier(high, low, close) * volume;
        self.flows.push_back(flow);
        if self.flows.len() > self.period {
            self.flows.pop_front();
        }
        if self.flows.len() < self.period {
            return None;
        }

        let positive: Decimal = self
            .flows
            .iter()
            .copied()
            .filter(|f| *f > Decimal::ZERO)
            .sum();
        let negative: Decimal = self
            .flows
            .iter()
            .copied()
            .filter(|f| *f < Decimal::ZERO)
            .map(|f| -f)
            .sum();
        let hundred = Decimal::from(100);
        let fifty = Decimal::from(50);
        let v = if negative.is_zero() {
            if positive.is_zero() { fifty } else { hundred }
        } else if positive.is_zero() {
            Decimal::ZERO
        } else {
            hundred - hundred / (Decimal::ONE + positive / negative)
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for Mfi {
    type Input<'a> = &'a tantra_core::Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<Decimal> {
        self.update_hlcv(
            bar.high.value(),
            bar.low.value(),
            bar.close.value(),
            bar.volume.value(),
        )
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.flows.clear();
        self.last = None;
    }
}
