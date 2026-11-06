//! Rate of Change (ROC).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// Rate of Change: `(close - close_n) / close_n * 100`, where `close_n` is the
/// price `period` periods ago.
///
/// The first value appears after `period + 1` inputs. A zero price `period`
/// periods back yields no output (division by zero is silently skipped).
#[derive(Clone, Debug)]
pub struct Roc {
    period: usize,
    buf: VecDeque<f64>,
    last: Option<f64>,
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
}

impl Indicator for Roc {
    type Input<'a> = f64;
    type Output = f64;

    fn update(&mut self, price: f64) -> Option<f64> {
        self.buf.push_back(price);
        if self.buf.len() > self.period + 1 {
            self.buf.pop_front();
        }
        if self.buf.len() < self.period + 1 {
            return None;
        }
        let prev = self.buf[self.buf.len() - 1 - self.period];
        if prev == 0.0 {
            return None;
        }
        let v = (price - prev) / prev * 100.0;
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.last = None;
    }
}
