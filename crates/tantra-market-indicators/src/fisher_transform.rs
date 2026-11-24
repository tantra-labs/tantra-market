//! Fisher Transform (Ehlers).

use std::collections::VecDeque;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, MathematicalOps};

use crate::indicator::Indicator;

/// Fisher Transform (John Ehlers), computed in `Decimal`.
///
/// Maps price into an approximately Gaussian (normal) series via
/// `0.5 * ln((1 + raw) / (1 − raw))`, where `raw` is the most recent price
/// rescaled into `[−1, 1]` against the `period`-bar range and exponentially
/// smoothed (`0.66` weight on the new reading, `0.67` on the previous raw
/// value, the classic 1/3–2/3 smoothing convention). The first value appears
/// after `period` inputs.
///
/// The output is largely cycle-free for trending data and is traditionally
/// used for divergences and extreme-zone (overbought/oversold) signals.
#[derive(Clone, Debug)]
pub struct FisherTransform {
    period: usize,
    buf: VecDeque<Decimal>,
    prev_raw: Decimal,
    last: Option<Decimal>,
}

impl FisherTransform {
    /// Creates a Fisher Transform over `period` samples.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "Fisher period must be positive");
        Self {
            period,
            buf: VecDeque::with_capacity(period),
            prev_raw: Decimal::ZERO,
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

impl Indicator for FisherTransform {
    type Input<'a> = Decimal;
    type Output = Decimal;

    fn update(&mut self, price: Decimal) -> Option<Decimal> {
        self.buf.push_back(price);
        if self.buf.len() > self.period {
            self.buf.pop_front();
        }
        if self.buf.len() < self.period {
            return None;
        }

        let min = self.buf.iter().copied().min().unwrap_or(price);
        let max = self.buf.iter().copied().max().unwrap_or(price);

        let raw = if max > min {
            let scaled = Decimal::TWO * (price - min) / (max - min) - Decimal::ONE;
            let raw = Decimal::new(66, 2) * scaled + Decimal::new(67, 2) * self.prev_raw;
            raw.clamp(Decimal::new(-999, 3), Decimal::new(999, 3))
        } else {
            self.prev_raw
        };
        self.prev_raw = raw;

        let ratio = (Decimal::ONE + raw) / (Decimal::ONE - raw);
        let v = Decimal::new(5, 1) * ratio.checked_ln().unwrap_or(Decimal::ZERO);
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.prev_raw = Decimal::ZERO;
        self.last = None;
    }
}
