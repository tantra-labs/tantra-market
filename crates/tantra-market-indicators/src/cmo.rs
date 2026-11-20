//! Chande Momentum Oscillator (CMO).

use std::collections::VecDeque;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::indicator::Indicator;

/// Chande Momentum Oscillator.
///
/// `100 * (Σgains − Σlosses) / (Σgains + Σlosses)` over the last `period`
/// price changes, where gains are up-moves and losses are down-moves (both
/// positive). The range is `[−100, 100]`; all-up moves give +100, all-down
/// moves give −100.
///
/// Computed in `Decimal` with running sums over a bounded window.
///
/// The first value appears after `period + 1` inputs (one extra to establish
/// the first price change).
#[derive(Clone, Debug)]
pub struct Cmo {
    period: usize,
    prev_price: Option<Decimal>,
    changes: VecDeque<(Decimal, Decimal)>,
    up: Decimal,
    down: Decimal,
    last: Option<Decimal>,
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
            changes: VecDeque::with_capacity(period + 1),
            up: Decimal::ZERO,
            down: Decimal::ZERO,
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// `f64` presentation shim over [`Indicator::update`].
    ///
    /// For chart/chat presentation surfaces only; financial math must use the `Decimal` input.
    /// Non-finite prices return `None` without mutating state.
    pub fn update_f64(&mut self, price: f64) -> Option<f64> {
        let p = Decimal::try_from(price).ok()?;
        self.update(p).and_then(|v| v.to_f64())
    }
}

impl Indicator for Cmo {
    type Input<'a> = Decimal;
    type Output = Decimal;

    fn update(&mut self, price: Decimal) -> Option<Decimal> {
        let prev = match self.prev_price {
            None => {
                self.prev_price = Some(price);
                return None;
            }
            Some(p) => p,
        };
        self.prev_price = Some(price);

        let change = price - prev;
        let (gain, loss) = if change > Decimal::ZERO {
            (change, Decimal::ZERO)
        } else if change < Decimal::ZERO {
            (Decimal::ZERO, -change)
        } else {
            (Decimal::ZERO, Decimal::ZERO)
        };
        self.changes.push_back((gain, loss));
        self.up += gain;
        self.down += loss;
        if self.changes.len() > self.period
            && let Some((og, ol)) = self.changes.pop_front()
        {
            self.up -= og;
            self.down -= ol;
        }
        if self.changes.len() < self.period {
            return None;
        }

        let total = self.up + self.down;
        let v = if total.is_zero() {
            Decimal::ZERO
        } else {
            Decimal::ONE_HUNDRED * (self.up - self.down) / total
        };
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.prev_price = None;
        self.changes.clear();
        self.up = Decimal::ZERO;
        self.down = Decimal::ZERO;
        self.last = None;
    }
}
