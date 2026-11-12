//! Fisher Transform (Ehlers).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// Fisher Transform (John Ehlers).
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
    buf: VecDeque<f64>,
    prev_raw: f64,
    last: Option<f64>,
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
            prev_raw: 0.0,
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }
}

impl Indicator for FisherTransform {
    type Input<'a> = f64;
    type Output = f64;

    fn update(&mut self, price: f64) -> Option<f64> {
        self.buf.push_back(price);
        if self.buf.len() > self.period {
            self.buf.pop_front();
        }
        if self.buf.len() < self.period {
            return None;
        }

        let min = self.buf.iter().copied().fold(f64::INFINITY, f64::min);
        let max = self.buf.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        let raw = if max > min {
            let scaled = 2.0 * (price - min) / (max - min) - 1.0;
            let raw = 0.66 * scaled + 0.67 * self.prev_raw;
            raw.clamp(-0.999, 0.999)
        } else {
            self.prev_raw
        };
        self.prev_raw = raw;

        let v = 0.5 * ((1.0 + raw) / (1.0 - raw)).ln();
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.prev_raw = 0.0;
        self.last = None;
    }
}
