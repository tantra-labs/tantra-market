//! Simple Moving Average.

use std::collections::VecDeque;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::indicator::Indicator;

/// Most samples reserved up front by [`Sma::new`].
const MAX_PREALLOC: usize = 4096;

/// A simple moving average over the last `period` inputs, computed in `Decimal`.
///
/// Uses a ring buffer and a running sum, so each update is O(1).
///
/// ```
/// use rust_decimal::Decimal;
/// use tantra_market_indicators::{Indicator, Sma};
///
/// let mut sma = Sma::new(3);
/// assert_eq!(sma.update(Decimal::from(1)), None);
/// assert_eq!(sma.update(Decimal::from(2)), None);
/// assert_eq!(sma.update(Decimal::from(3)), Some(Decimal::from(2)));
/// assert_eq!(sma.update(Decimal::from(4)), Some(Decimal::from(3)));
/// ```
#[derive(Clone, Debug)]
pub struct Sma {
    period: usize,
    buf: VecDeque<Decimal>,
    sum: Decimal,
    last: Option<Decimal>,
}

impl Sma {
    /// Creates an SMA over `period` samples.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "SMA period must be positive");
        Self {
            period,
            // Capped so an absurd period cannot abort on allocation; the
            // buffer grows on demand beyond this.
            buf: VecDeque::with_capacity(period.min(MAX_PREALLOC)),
            sum: Decimal::ZERO,
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

impl Indicator for Sma {
    type Input<'a> = Decimal;
    type Output = Decimal;

    fn update(&mut self, input: Decimal) -> Option<Decimal> {
        self.buf.push_back(input);
        self.sum += input;
        if self.buf.len() > self.period
            && let Some(old) = self.buf.pop_front()
        {
            self.sum -= old;
        }
        if self.buf.len() == self.period {
            let v = self.sum / Decimal::from(self.period);
            self.last = Some(v);
            Some(v)
        } else {
            None
        }
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.sum = Decimal::ZERO;
        self.last = None;
    }
}
