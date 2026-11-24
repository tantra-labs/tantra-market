//! Mass Index (Donald Dorsey).

use std::collections::VecDeque;

use rust_decimal::Decimal;

use crate::ema::Ema;
use crate::indicator::Indicator;

/// Mass Index, computed in `Decimal`.
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
    ratios: VecDeque<Decimal>,
    last: Option<Decimal>,
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

    /// Feeds one bar given as raw `Decimal` `high`/`low` and returns the Mass Index once
    /// warmed up. Identical to [`Indicator::update`] on a bar with those
    /// fields; the close is unused.
    pub fn update_hl(&mut self, high: Decimal, low: Decimal) -> Option<Decimal> {
        let range = (high - low).max(Decimal::ZERO);
        let e1 = self.ema_range.update(range)?;
        let e2 = self.ema_ema_range.update(e1)?;
        let ratio = if e2.is_zero() { Decimal::ZERO } else { e1 / e2 };

        self.ratios.push_back(ratio);
        if self.ratios.len() > self.period {
            self.ratios.pop_front();
        }
        if self.ratios.len() < self.period {
            return None;
        }
        let v: Decimal = self.ratios.iter().copied().sum();
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for MassIndex {
    type Input<'a> = &'a tantra_core::Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<Decimal> {
        self.update_hl(bar.high.value(), bar.low.value())
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.ema_range.reset();
        self.ema_ema_range.reset();
        self.ratios.clear();
        self.last = None;
    }
}
