//! Chandelier Exit (volatility trailing stop indicator).
//!
//! Computes both long and short Chandelier trailing levels in exact `Decimal`:
//! - `long_stop = highest_high(period) - mult * ATR(period)`
//! - `short_stop = lowest_low(period) + mult * ATR(period)`

use std::collections::VecDeque;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tantra_core::Bar;

use crate::atr::Atr;
use crate::indicator::Indicator;

/// Output of [`ChandelierExit`]: trailing stops for long and short positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChandelierExitValue {
    /// Trailing stop for a long position: `highest_high(period) - mult * ATR(period)`.
    pub long_stop: Decimal,
    /// Trailing stop for a short position: `lowest_low(period) + mult * ATR(period)`.
    pub short_stop: Decimal,
}

/// Streaming Chandelier Exit over `period` bars with ATR multiplier `mult`, in `Decimal`.
///
/// Reuses [`Atr`] for Wilder's ATR and maintains O(1)-amortized monotonic deques for the
/// rolling highest high and lowest low. Returns `None` until `period` bars have been processed.
#[derive(Clone, Debug)]
pub struct ChandelierExit {
    period: usize,
    mult: Decimal,
    atr: Atr,
    index: usize,
    highs: VecDeque<(usize, Decimal)>,
    lows: VecDeque<(usize, Decimal)>,
    last: Option<ChandelierExitValue>,
}

impl ChandelierExit {
    /// Creates a Chandelier Exit over `period` bars with ATR multiplier `mult`.
    ///
    /// Canonical parameters are `period = 22` and `mult = 3`.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero or `mult` is negative.
    pub fn new(period: usize, mult: Decimal) -> Self {
        assert!(period > 0, "ChandelierExit period must be positive");
        assert!(
            mult >= Decimal::ZERO,
            "ChandelierExit multiplier must be non-negative"
        );
        Self {
            period,
            mult,
            atr: Atr::new(period),
            index: 0,
            highs: VecDeque::with_capacity(period),
            lows: VecDeque::with_capacity(period),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Returns the configured ATR multiplier.
    pub fn mult(&self) -> Decimal {
        self.mult
    }

    /// Feeds one bar given as raw `high`, `low`, `close` and returns stops once warmed up.
    pub fn update_hlc(
        &mut self,
        high: Decimal,
        low: Decimal,
        close: Decimal,
    ) -> Option<ChandelierExitValue> {
        let idx = self.index;
        self.index += 1;

        while self.highs.back().is_some_and(|(_, v)| *v <= high) {
            self.highs.pop_back();
        }
        self.highs.push_back((idx, high));

        while self.lows.back().is_some_and(|(_, v)| *v >= low) {
            self.lows.pop_back();
        }
        self.lows.push_back((idx, low));

        let cutoff = idx.saturating_sub(self.period - 1);
        while self.highs.front().is_some_and(|(i, _)| *i < cutoff) {
            self.highs.pop_front();
        }
        while self.lows.front().is_some_and(|(i, _)| *i < cutoff) {
            self.lows.pop_front();
        }

        let atr_val = self.atr.update_hlc(high, low, close)?;
        if idx + 1 < self.period {
            return None;
        }

        let highest_high = self.highs.front().map(|(_, v)| *v)?;
        let lowest_low = self.lows.front().map(|(_, v)| *v)?;
        let offset = self.mult * atr_val;
        let v = ChandelierExitValue {
            long_stop: highest_high - offset,
            short_stop: lowest_low + offset,
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for ChandelierExit {
    type Input<'a> = &'a Bar;
    type Output = ChandelierExitValue;

    fn update(&mut self, bar: &Bar) -> Option<ChandelierExitValue> {
        self.update_hlc(bar.high.value(), bar.low.value(), bar.close.value())
    }

    fn value(&self) -> Option<ChandelierExitValue> {
        self.last
    }

    fn reset(&mut self) {
        self.atr.reset();
        self.index = 0;
        self.highs.clear();
        self.lows.clear();
        self.last = None;
    }
}
