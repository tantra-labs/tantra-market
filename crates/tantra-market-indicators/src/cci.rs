//! Commodity Channel Index (CCI).

use std::collections::VecDeque;

use rust_decimal::Decimal;

use crate::indicator::Indicator;

/// Commodity Channel Index (Lambert), computed in `Decimal`.
///
/// `CCI = (TP − SMA(TP, n)) / (0.015 * mean_deviation)`, where
/// `TP = (H + L + C) / 3` and the mean deviation is the average of
/// `|TP − SMA|` over the same window. The first value appears after `period`
/// bars. Extremes beyond ±100 (or ±200) are the classic overbought/oversold
/// signals.
#[derive(Clone, Debug)]
pub struct Cci {
    period: usize,
    tps: VecDeque<Decimal>,
    last: Option<Decimal>,
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

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let three = Decimal::from(3);
        let tp = (high + low + close) / three;
        self.tps.push_back(tp);
        if self.tps.len() > self.period {
            self.tps.pop_front();
        }
        if self.tps.len() < self.period {
            return None;
        }

        let n = Decimal::from(self.period);
        let sma = self.tps.iter().copied().sum::<Decimal>() / n;
        let md = self.tps.iter().map(|&t| (t - sma).abs()).sum::<Decimal>() / n;
        let scale = Decimal::new(15, 3); // 0.015
        let v = if md.is_zero() {
            Decimal::ZERO
        } else {
            (tp - sma) / (scale * md)
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for Cci {
    type Input<'a> = &'a tantra_core::Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<Decimal> {
        self.update_hlc(bar.high.value(), bar.low.value(), bar.close.value())
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.tps.clear();
        self.last = None;
    }
}
