//! Rate of Change (ROC).

use std::collections::VecDeque;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::indicator::Indicator;

/// Rate of Change, computed in `Decimal`: `(close - close_n) / close_n * 100`,
/// where `close_n` is the price `period` periods ago.
///
/// The first value appears after `period + 1` inputs. A zero price `period`
/// periods back yields no output (division by zero is silently skipped).
#[derive(Clone, Debug)]
pub struct Roc {
    period: usize,
    buf: VecDeque<Decimal>,
    last: Option<Decimal>,
}

impl Roc {
    /// Creates a ROC over `period` price changes.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "ROC period must be positive");
        Self {
            period,
            buf: VecDeque::with_capacity(period + 1),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
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

impl Indicator for Roc {
    type Input<'a> = Decimal;
    type Output = Decimal;

    fn update(&mut self, price: Decimal) -> Option<Decimal> {
        self.buf.push_back(price);
        if self.buf.len() > self.period + 1 {
            self.buf.pop_front();
        }
        if self.buf.len() < self.period + 1 {
            return None;
        }
        let prev = self.buf[self.buf.len() - 1 - self.period];
        if prev.is_zero() {
            return None;
        }
        let v = (price - prev) / prev * Decimal::ONE_HUNDRED;
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.last = None;
    }
}
