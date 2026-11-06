//! Chande Momentum Oscillator (CMO).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// Chande Momentum Oscillator.
///
/// `100 * (Σgains − Σlosses) / (Σgains + Σlosses)` over the last `period`
/// price changes, where gains are up-moves and losses are down-moves (both
/// positive). The range is `[−100, 100]`; all-up moves give +100, all-down
/// moves give −100.
///
/// The first value appears after `period + 1` inputs (one extra to establish
/// the first price change).
#[derive(Clone, Debug)]
pub struct Cmo {
    period: usize,
    prev_price: Option<f64>,
    changes: VecDeque<(f64, f64)>,
    last: Option<f64>,
}

impl Cmo {
    /// Creates a CMO over `period` price changes.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "CMO period must be positive");
        Self {
            period,
            prev_price: None,
            changes: VecDeque::with_capacity(period),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }
}

impl Indicator for Cmo {
    type Input<'a> = f64;
    type Output = f64;

    fn update(&mut self, price: f64) -> Option<f64> {
        let prev = match self.prev_price {
            None => {
                self.prev_price = Some(price);
                return None;
            }
            Some(p) => p,
        };
        self.prev_price = Some(price);

        let change = price - prev;
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);
        self.changes.push_back((gain, loss));
        if self.changes.len() > self.period {
            self.changes.pop_front();
        }
        if self.changes.len() < self.period {
            return None;
        }

        let sum_gain: f64 = self.changes.iter().map(|(g, _)| g).sum();
        let sum_loss: f64 = self.changes.iter().map(|(_, l)| l).sum();
        let denom = sum_gain + sum_loss;
        let v = if denom == 0.0 {
            0.0
        } else {
            (sum_gain - sum_loss) / denom * 100.0
        };
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.prev_price = None;
        self.changes.clear();
        self.last = None;
    }
}
