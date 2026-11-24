//! TRIX: triple-smoothed exponential moving average, as a percentage change.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::ema::Ema;
use crate::indicator::Indicator;

/// TRIX, computed in `Decimal`.
///
/// `100 * (ema3 − ema3_prev) / ema3_prev`, where `ema3` is an EMA of an EMA of
/// an EMA of price, all over the same `period`. The first value appears once
/// all three EMAs have primed (`3 * period − 2` inputs) *and* a second `ema3`
/// exists to form the percentage change (`3 * period − 1` inputs).
#[derive(Clone, Debug)]
pub struct Trix {
    ema1: Ema,
    ema2: Ema,
    ema3: Ema,
    prev_ema3: Option<Decimal>,
    last: Option<Decimal>,
}

impl Trix {
    /// Creates a TRIX over `period`.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "TRIX period must be positive");
        Self {
            ema1: Ema::new(period),
            ema2: Ema::new(period),
            ema3: Ema::new(period),
            prev_ema3: None,
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.ema1.period()
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

impl Indicator for Trix {
    type Input<'a> = Decimal;
    type Output = Decimal;

    fn update(&mut self, price: Decimal) -> Option<Decimal> {
        // Every EMA must see every input, even before the previous stage has
        // primed — using `?` would starve the downstream stages.
        let e1 = self.ema1.update(price);
        let e2 = self.ema2.update(e1?);
        let e3 = self.ema3.update(e2?)?;

        let prev = match self.prev_ema3 {
            None => {
                self.prev_ema3 = Some(e3);
                return None;
            }
            Some(p) => p,
        };
        self.prev_ema3 = Some(e3);

        let v = if prev.is_zero() {
            Decimal::ZERO
        } else {
            (e3 - prev) / prev * Decimal::ONE_HUNDRED
        };
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.ema1.reset();
        self.ema2.reset();
        self.ema3.reset();
        self.prev_ema3 = None;
        self.last = None;
    }
}
