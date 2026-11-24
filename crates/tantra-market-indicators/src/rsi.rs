//! Relative Strength Index (Wilder).

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::indicator::Indicator;

/// Wilder's Relative Strength Index, computed in `Decimal`.
///
/// Produces values in `[0, 100]`. The first value appears after `period + 1`
/// inputs (one extra to establish the first price change). Subsequent values
/// use Wilder smoothing:
///
/// `avg = (avg * (n - 1) + x) / n`
///
/// ```
/// use rust_decimal::Decimal;
/// use tantra_market_indicators::{Indicator, Rsi};
///
/// let mut rsi = Rsi::new(3);
/// for p in [1, 2, 3, 4] {
///     rsi.update(Decimal::from(p));
/// }
/// // All-up moves drive RSI to 100.
/// assert_eq!(rsi.value(), Some(Decimal::ONE_HUNDRED));
/// ```
#[derive(Clone, Debug)]
pub struct Rsi {
    period: usize,
    prev_price: Option<Decimal>,
    seed_gains: Vec<Decimal>,
    seed_losses: Vec<Decimal>,
    avg_gain: Option<Decimal>,
    avg_loss: Option<Decimal>,
    last: Option<Decimal>,
}

impl Rsi {
    /// Creates an RSI over `period` price changes.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "RSI period must be positive");
        Self {
            period,
            prev_price: None,
            seed_gains: Vec::with_capacity(period),
            seed_losses: Vec::with_capacity(period),
            avg_gain: None,
            avg_loss: None,
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

    fn compute(avg_gain: Decimal, avg_loss: Decimal) -> Decimal {
        if avg_loss.is_zero() {
            Decimal::ONE_HUNDRED
        } else {
            let rs = avg_gain / avg_loss;
            Decimal::ONE_HUNDRED - Decimal::ONE_HUNDRED / (Decimal::ONE + rs)
        }
    }
}

impl Indicator for Rsi {
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
        let gain = if change > Decimal::ZERO {
            change
        } else {
            Decimal::ZERO
        };
        let loss = if change < Decimal::ZERO {
            -change
        } else {
            Decimal::ZERO
        };

        let n = Decimal::from(self.period);
        if self.avg_gain.is_none() {
            self.seed_gains.push(gain);
            self.seed_losses.push(loss);
            if self.seed_gains.len() == self.period {
                let g: Decimal = self.seed_gains.iter().copied().sum::<Decimal>() / n;
                let l: Decimal = self.seed_losses.iter().copied().sum::<Decimal>() / n;
                self.seed_gains.clear();
                self.seed_losses.clear();
                self.avg_gain = Some(g);
                self.avg_loss = Some(l);
                let v = Self::compute(g, l);
                self.last = Some(v);
                return Some(v);
            }
            return None;
        }

        let g = (self.avg_gain.unwrap() * (n - Decimal::ONE) + gain) / n;
        let l = (self.avg_loss.unwrap() * (n - Decimal::ONE) + loss) / n;
        self.avg_gain = Some(g);
        self.avg_loss = Some(l);
        let v = Self::compute(g, l);
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.prev_price = None;
        self.seed_gains.clear();
        self.seed_losses.clear();
        self.avg_gain = None;
        self.avg_loss = None;
        self.last = None;
    }
}
