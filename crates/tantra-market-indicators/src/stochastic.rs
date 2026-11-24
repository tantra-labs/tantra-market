//! Stochastic oscillator (%K and %D).
//!
//! Raw `%K = 100 * (close - LL) / (HH - LL)` over `period` bars. `HH` is the
//! highest high and `LL` the lowest low in the window. `HH == LL` yields
//! `50` by convention (neutral). `%D` is the SMA of the last `d_period`
//! `%K` values, `None` until that SMA is primed.

use std::collections::VecDeque;

use rust_decimal::Decimal;
use tantra_core::Bar;

use crate::indicator::Indicator;

/// The value produced by [`Stochastic`], computed in `Decimal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StochasticValue {
    /// Raw `%K`, `0..100` (flat window gives `50`).
    pub k: Decimal,
    /// Smoothed `%D`, an SMA of `%K` over `d_period`.
    pub d: Option<Decimal>,
}

/// Stochastic oscillator (%K and %D) over `period` bars and a `d_period` SMA,
/// computed in `Decimal`.
///
/// Monotonic deques give O(1) amortised updates; the keepers of `k` values
/// and their running sum give O(1) for `d`.
///
/// ```
/// use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
/// use tantra_market_indicators::{Indicator, Stochastic};
/// use rust_decimal::Decimal;
///
/// fn bar(h: i64, l: i64, c: i64) -> Bar {
///     Bar {
///         instrument: InstrumentId::try_new(Exchange::Nse, "X").unwrap(),
///         open: Price::try_new(Decimal::from(c)).unwrap(),
///         high: Price::try_new(Decimal::from(h)).unwrap(),
///         low: Price::try_new(Decimal::from(l)).unwrap(),
///         close: Price::try_new(Decimal::from(c)).unwrap(),
///         volume: Quantity::from_u64(1),
///         timestamp: chrono::Utc::now(),
///     }
/// }
/// let mut s = Stochastic::new(3, 2);
/// s.update(&bar(10, 5, 7));
/// s.update(&bar(11, 6, 9));
/// let v = s.update(&bar(9, 4, 6)).unwrap();
/// // HH=11, LL=4, close=6 -> k = 100*(2/7) ~ 28.571428...
/// assert!((v.k - Decimal::from(200) / Decimal::from(7)).abs() < Decimal::new(1, 9));
/// assert!(v.d.is_none());
/// let v2 = s.update(&bar(12, 7, 10)).unwrap();
/// // Next k = 75, so d = (200/7 + 75) / 2
/// let expected_d = (Decimal::from(200) / Decimal::from(7) + Decimal::from(75)) / Decimal::TWO;
/// assert!((v2.d.unwrap() - expected_d).abs() < Decimal::new(1, 9));
/// ```
#[derive(Clone, Debug)]
pub struct Stochastic {
    period: usize,
    d_period: usize,
    index: usize,
    highs: VecDeque<(usize, Decimal)>,
    lows: VecDeque<(usize, Decimal)>,
    ks: VecDeque<Decimal>,
    k_sum: Decimal,
    last: Option<StochasticValue>,
}

impl Stochastic {
    /// Creates a stochastic oscillator over `period` bars with a `d_period` SMA.
    ///
    /// # Panics
    ///
    /// Panics if either period is zero.
    pub fn new(period: usize, d_period: usize) -> Self {
        assert!(period > 0, "Stochastic period must be positive");
        assert!(d_period > 0, "Stochastic d_period must be positive");
        Self {
            period,
            d_period,
            index: 0,
            highs: VecDeque::new(),
            lows: VecDeque::new(),
            ks: VecDeque::new(),
            k_sum: Decimal::ZERO,
            last: None,
        }
    }

    /// Returns the primary `%K` lookback.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Returns the smoothing lookback for `%D`.
    pub fn d_period(&self) -> usize {
        self.d_period
    }

    fn push(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<StochasticValue> {
        let idx = self.index;
        self.index += 1;

        while self.highs.back().is_some_and(|(_, v)| *v <= high) {
            self.highs.pop_back();
        }
        self.highs.push_back((idx, high));

        while self.lows.back().is_some_and(|(_, v)| *v >= low) {
            self.lows.pop_back();
        }
        self.lows.push_back((idx, low));

        let cutoff = idx.saturating_sub(self.period - 1);
        while self.highs.front().is_some_and(|(i, _)| *i < cutoff) {
            self.highs.pop_front();
        }
        while self.lows.front().is_some_and(|(i, _)| *i < cutoff) {
            self.lows.pop_front();
        }

        if idx + 1 < self.period {
            return None;
        }

        let hh = self.highs.front().unwrap().1;
        let ll = self.lows.front().unwrap().1;
        let denom = hh - ll;
        let k = if denom.is_zero() {
            Decimal::from(50)
        } else {
            Decimal::from(100) * (close - ll) / denom
        };

        self.ks.push_back(k);
        self.k_sum += k;
        if self.ks.len() > self.d_period
            && let Some(old) = self.ks.pop_front()
        {
            self.k_sum -= old;
        }

        let d = if self.ks.len() == self.d_period {
            Some(self.k_sum / Decimal::from(self.d_period))
        } else {
            None
        };

        let v = StochasticValue { k, d };
        self.last = Some(v);
        Some(v)
    }

    /// Feeds raw `Decimal` `high`/`low`/`close` without building a `Bar`.
    pub fn update_hlc(
        &mut self,
        high: Decimal,
        low: Decimal,
        close: Decimal,
    ) -> Option<StochasticValue> {
        self.push(high, low, close)
    }
}

impl Indicator for Stochastic {
    type Input<'a> = &'a Bar;
    type Output = StochasticValue;

    fn update(&mut self, bar: &Bar) -> Option<StochasticValue> {
        self.push(bar.high.value(), bar.low.value(), bar.close.value())
    }

    fn value(&self) -> Option<StochasticValue> {
        self.last
    }

    fn reset(&mut self) {
        self.index = 0;
        self.highs.clear();
        self.lows.clear();
        self.ks.clear();
        self.k_sum = Decimal::ZERO;
        self.last = None;
    }
}
