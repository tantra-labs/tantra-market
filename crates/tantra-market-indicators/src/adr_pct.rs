//! Average Daily Range Percent (ADR%).
//!
//! Computes the Qullamaggie FAQ formula in exact `Decimal`:
//! `100 * (mean(high_i / low_i) - 1)` over the trailing `period` bars
//! (average of daily high/low ratios, not ratio of averages).

use std::collections::VecDeque;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tantra_core::Bar;

use crate::indicator::Indicator;

/// Streaming Average Daily Range Percent (`ADR%`) over `period` bars, in `Decimal`.
///
/// Returns `None` while fewer than `period` valid bars have been observed.
#[derive(Clone, Debug)]
pub struct AdrPct {
    period: usize,
    ratios: VecDeque<Decimal>,
    last: Option<Decimal>,
}

impl AdrPct {
    /// Creates an `AdrPct` over `period` bars (20 is the Qullamaggie canonical period).
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "ADR% period must be positive");
        Self {
            period,
            ratios: VecDeque::with_capacity(period),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Feeds one bar given as raw `high` and `low` prices and returns `ADR%` once warmed up.
    ///
    /// Non-positive `low` values cannot form a `high / low` ratio and return `None` without
    /// mutating state.
    pub fn update_hl(&mut self, high: Decimal, low: Decimal) -> Option<Decimal> {
        if low <= Decimal::ZERO {
            return None;
        }
        let ratio = high / low;
        if self.ratios.len() == self.period {
            self.ratios.pop_front();
        }
        self.ratios.push_back(ratio);

        if self.ratios.len() < self.period {
            return None;
        }

        let sum: Decimal = self.ratios.iter().copied().sum();
        let mean_ratio = sum / Decimal::from(self.period);
        let pct = (mean_ratio - Decimal::ONE) * Decimal::ONE_HUNDRED;
        self.last = Some(pct);
        Some(pct)
    }

    /// `f64` presentation shim over [`AdrPct::update_hl`].
    ///
    /// For chart presentation surfaces only; financial math must use [`AdrPct::update_hl`].
    /// Non-finite inputs return `None` without mutating state.
    pub fn update_hl_f64(&mut self, high: f64, low: f64) -> Option<f64> {
        let h = Decimal::try_from(high).ok()?;
        let l = Decimal::try_from(low).ok()?;
        self.update_hl(h, l).and_then(|v| v.to_f64())
    }
}

impl Indicator for AdrPct {
    type Input<'a> = &'a Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &Bar) -> Option<Decimal> {
        self.update_hl(bar.high.0, bar.low.0)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.ratios.clear();
        self.last = None;
    }
}
