//! Bollinger Bands.

use std::collections::VecDeque;

use rust_decimal::{Decimal, MathematicalOps};

use crate::indicator::Indicator;

/// The three bands produced by [`BollingerBands`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BollingerValue {
    /// The simple moving average.
    pub middle: Decimal,
    /// `middle + k * stddev`.
    pub upper: Decimal,
    /// `middle - k * stddev`.
    pub lower: Decimal,
}

/// Bollinger Bands over `period` samples with `k` standard deviations, computed in `Decimal`.
///
/// Uses population standard deviation (`n` in the denominator). This matches
/// the convention used by most charting platforms.
///
/// ```
/// use rust_decimal::{Decimal, MathematicalOps};
/// use tantra_market_indicators::{BollingerBands, Indicator};
///
/// let mut bb = BollingerBands::new(5, Decimal::TWO);
/// for x in [1, 2, 3, 4, 5] {
///     bb.update(Decimal::from(x));
/// }
/// let v = bb.value().unwrap();
/// assert_eq!(v.middle, Decimal::from(3));
/// // population stddev = sqrt(2) ≈ 1.41421356...
/// let expected = Decimal::from(3) + Decimal::TWO * Decimal::TWO.sqrt().unwrap();
/// assert!((v.upper - expected).abs() < Decimal::new(1, 9));
/// ```
#[derive(Clone, Debug)]
pub struct BollingerBands {
    period: usize,
    k: Decimal,
    buf: VecDeque<Decimal>,
    sum: Decimal,
    last: Option<BollingerValue>,
}

impl BollingerBands {
    /// Creates Bollinger Bands.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero or `k` is negative.
    pub fn new(period: usize, k: Decimal) -> Self {
        assert!(period > 0, "Bollinger period must be positive");
        assert!(k >= Decimal::ZERO, "Bollinger k must be non-negative");
        Self {
            period,
            k,
            buf: VecDeque::with_capacity(period),
            sum: Decimal::ZERO,
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Returns the configured standard-deviation multiplier.
    pub fn k(&self) -> Decimal {
        self.k
    }
}

impl Indicator for BollingerBands {
    type Input<'a> = Decimal;
    type Output = BollingerValue;

    fn update(&mut self, input: Decimal) -> Option<BollingerValue> {
        self.buf.push_back(input);
        self.sum += input;
        if self.buf.len() > self.period
            && let Some(old) = self.buf.pop_front()
        {
            self.sum -= old;
        }
        if self.buf.len() < self.period {
            return None;
        }

        let n = Decimal::from(self.period);
        let mean = self.sum / n;
        let variance = self
            .buf
            .iter()
            .map(|&x| {
                let d = x - mean;
                d * d
            })
            .sum::<Decimal>()
            / n;
        let std = variance.sqrt().unwrap_or(Decimal::ZERO);

        let v = BollingerValue {
            middle: mean,
            upper: mean + self.k * std,
            lower: mean - self.k * std,
        };
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<BollingerValue> {
        self.last
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.sum = Decimal::ZERO;
        self.last = None;
    }
}
