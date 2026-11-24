//! RMA (Running Moving Average / Wilder's Smoothing).
//!
//! RMA is Wilder's smoothing method used in RSI, ATR, and ADX calculations.
//! It gives more weight to recent values.
//!
//! # Formula
//!
//! ```text
//! RMA = (PrevRMA * (Period - 1) + Current) / Period
//! ```

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::indicator::Indicator;

/// Most seed samples reserved up front by [`Rma::new`].
const MAX_PREALLOC: usize = 4096;

/// Running Moving Average (Wilder's Smoothing), computed in `Decimal`.
#[derive(Clone, Debug)]
pub struct Rma {
    period: usize,
    seed: Vec<Decimal>,
    sum: Decimal,
    rma: Option<Decimal>,
}

impl Rma {
    /// Creates a new RMA with the given period.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "RMA period must be positive");
        Self {
            period,
            seed: Vec::with_capacity(period.min(MAX_PREALLOC)),
            sum: Decimal::ZERO,
            rma: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Returns the current RMA value.
    pub fn value(&self) -> Option<Decimal> {
        self.rma
    }

    /// `f64` presentation shim over [`Indicator::update`].
    ///
    /// For chart presentation surfaces only; financial math must use the `Decimal` input.
    /// Non-finite inputs return `None` without mutating state.
    pub fn update_f64(&mut self, price: f64) -> Option<f64> {
        let p = Decimal::try_from(price).ok()?;
        self.update(p).and_then(|v| v.to_f64())
    }
}

impl Default for Rma {
    fn default() -> Self {
        Self::new(14)
    }
}

impl Indicator for Rma {
    type Input<'a> = Decimal;
    type Output = Decimal;

    fn update(&mut self, input: Decimal) -> Option<Decimal> {
        let n = Decimal::from(self.period);
        match self.rma {
            None => {
                self.seed.push(input);
                self.sum += input;
                if self.seed.len() < self.period {
                    return None;
                }
                let first = self.sum / n;
                self.seed.clear();
                self.rma = Some(first);
                Some(first)
            }
            Some(prev) => {
                let next = (prev * (n - Decimal::ONE) + input) / n;
                self.rma = Some(next);
                Some(next)
            }
        }
    }

    fn value(&self) -> Option<Decimal> {
        self.rma
    }

    fn reset(&mut self) {
        self.seed.clear();
        self.sum = Decimal::ZERO;
        self.rma = None;
    }
}
