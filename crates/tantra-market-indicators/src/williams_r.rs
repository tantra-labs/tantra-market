//! Williams %R.
//!
//! `%R = -100 * (HH - close) / (HH - LL)` over `period` bars. `HH` is the
//! highest high and `LL` the lowest low in the window. A flat window
//! (`HH == LL`) yields `-50` by convention.

use std::collections::VecDeque;

use rust_decimal::Decimal;
use tantra_core::Bar;

use crate::indicator::Indicator;

/// Williams %R over `period` bars, in `[-100, 0]`, computed in `Decimal`.
///
/// Monotonic deques give O(1) amortised updates.
///
/// ```
/// use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
/// use tantra_market_indicators::{Indicator, WilliamsR};
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
/// let mut w = WilliamsR::new(3);
/// w.update(&bar(10, 5, 9));
/// w.update(&bar(12, 6, 11));
/// let r = w.update(&bar(11, 4, 6)).unwrap();
/// // HH=12, LL=4, close=6 -> -100*(6/8) = -75
/// assert_eq!(r, Decimal::from(-75));
/// ```
#[derive(Clone, Debug)]
pub struct WilliamsR {
    period: usize,
    index: usize,
    highs: VecDeque<(usize, Decimal)>,
    lows: VecDeque<(usize, Decimal)>,
    last: Option<Decimal>,
}

impl WilliamsR {
    /// Creates a Williams %R over `period` bars.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "WilliamsR period must be positive");
        Self {
            period,
            index: 0,
            highs: VecDeque::new(),
            lows: VecDeque::new(),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    fn push(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
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

        if idx + 1 >= self.period {
            let hh = self.highs.front().unwrap().1;
            let ll = self.lows.front().unwrap().1;
            let denom = hh - ll;
            let v = if denom.is_zero() {
                Decimal::from(-50)
            } else {
                Decimal::from(-100) * (hh - close) / denom
            };
            self.last = Some(v);
            Some(v)
        } else {
            None
        }
    }

    /// Feeds raw `Decimal` `high`/`low`/`close` without building a `Bar`.
    pub fn update_hlc(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        self.push(high, low, close)
    }
}

impl Indicator for WilliamsR {
    type Input<'a> = &'a Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &Bar) -> Option<Decimal> {
        self.push(bar.high.value(), bar.low.value(), bar.close.value())
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.index = 0;
        self.highs.clear();
        self.lows.clear();
        self.last = None;
    }
}
