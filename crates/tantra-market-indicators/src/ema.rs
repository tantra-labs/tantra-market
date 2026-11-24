//! Exponential Moving Average.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::indicator::Indicator;
use crate::sma::Sma;

/// An exponential moving average, computed in `Decimal`.
///
/// Seeded with the SMA of the first `period` inputs, then smoothed using
/// `alpha = 2 / (period + 1)`. This is the standard formulation used by most
/// charting platforms; it converges to the same values as the recursive
/// form once the seed effect decays.
///
/// ```
/// use rust_decimal::Decimal;
/// use tantra_market_indicators::{Ema, Indicator};
///
/// let mut ema = Ema::new(3);
/// assert_eq!(ema.update(Decimal::from(1)), None);
/// assert_eq!(ema.update(Decimal::from(2)), None);
/// assert_eq!(ema.update(Decimal::from(3)), Some(Decimal::from(2)));   // SMA seed = (1+2+3)/3
/// assert_eq!(ema.update(Decimal::from(4)), Some(Decimal::from(3)));   // 0.5*4 + 0.5*2
/// assert_eq!(ema.update(Decimal::from(5)), Some(Decimal::from(4)));   // 0.5*5 + 0.5*3
/// ```
#[derive(Clone, Debug)]
pub struct Ema {
    period: usize,
    alpha: Decimal,
    seed: Sma,
    prev: Option<Decimal>,
}

impl Ema {
    /// Creates an EMA over `period` samples.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "EMA period must be positive");
        Self {
            period,
            alpha: Decimal::TWO / (Decimal::from(period) + Decimal::ONE),
            seed: Sma::new(period),
            prev: None,
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

impl Indicator for Ema {
    type Input<'a> = Decimal;
    type Output = Decimal;

    fn update(&mut self, input: Decimal) -> Option<Decimal> {
        if let Some(prev) = self.prev {
            let next = self.alpha * input + (Decimal::ONE - self.alpha) * prev;
            self.prev = Some(next);
            Some(next)
        } else if let Some(seed) = self.seed.update(input) {
            self.prev = Some(seed);
            Some(seed)
        } else {
            None
        }
    }

    fn value(&self) -> Option<Decimal> {
        self.prev
    }

    fn reset(&mut self) {
        self.seed.reset();
        self.prev = None;
    }
}
