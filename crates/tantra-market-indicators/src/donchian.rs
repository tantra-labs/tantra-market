//! Donchian Channel.

use std::collections::VecDeque;

use rust_decimal::Decimal;
use tantra_core::Bar;

use crate::indicator::Indicator;

/// Output of the Donchian Channel indicator, computed in `Decimal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DonchianValue {
    /// Highest high over the window.
    pub upper: Decimal,
    /// Lowest low over the window.
    pub lower: Decimal,
    /// Midpoint `(upper + lower) / 2`.
    pub middle: Decimal,
}

/// Donchian Channel over `period` bars, computed in `Decimal`.
///
/// Each update is O(1) amortised via monotonic deques for the rolling
/// maximum of highs and minimum of lows.
///
/// ```
/// use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
/// use tantra_market_indicators::{Donchian, Indicator};
/// use rust_decimal::Decimal;
///
/// fn bar(h: i64, l: i64) -> Bar {
///     Bar {
///         instrument: InstrumentId::try_new(Exchange::Nse, "X").unwrap(),
///         open: Price::try_new(Decimal::from(l)).unwrap(),
///         high: Price::try_new(Decimal::from(h)).unwrap(),
///         low: Price::try_new(Decimal::from(l)).unwrap(),
///         close: Price::try_new(Decimal::from(l)).unwrap(),
///         volume: Quantity::from_u64(1),
///         timestamp: chrono::Utc::now(),
///     }
/// }
/// let mut d = Donchian::new(3);
/// assert_eq!(d.update(&bar(10, 8)), None);
/// assert_eq!(d.update(&bar(11, 9)), None);
/// let v = d.update(&bar(9, 7)).unwrap();
/// assert_eq!(v.upper, Decimal::from(11));
/// assert_eq!(v.lower, Decimal::from(7));
/// ```
#[derive(Clone, Debug)]
pub struct Donchian {
    period: usize,
    index: usize,
    highs: VecDeque<(usize, Decimal)>,
    lows: VecDeque<(usize, Decimal)>,
    last: Option<DonchianValue>,
}

impl Donchian {
    /// Creates a Donchian Channel over `period` bars.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "Donchian period must be positive");
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

    fn push(&mut self, high: Decimal, low: Decimal) -> Option<DonchianValue> {
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
            let upper = self.highs.front().unwrap().1;
            let lower = self.lows.front().unwrap().1;
            let v = DonchianValue {
                upper,
                lower,
                middle: (upper + lower) / Decimal::TWO,
            };
            self.last = Some(v);
            Some(v)
        } else {
            None
        }
    }

    /// Feeds raw `Decimal` `high`/`low` without building a `Bar`.
    pub fn update_hl(&mut self, high: Decimal, low: Decimal) -> Option<DonchianValue> {
        self.push(high, low)
    }
}

impl Indicator for Donchian {
    type Input<'a> = &'a Bar;
    type Output = DonchianValue;

    fn update(&mut self, bar: &Bar) -> Option<DonchianValue> {
        self.push(bar.high.value(), bar.low.value())
    }

    fn value(&self) -> Option<DonchianValue> {
        self.last
    }

    fn reset(&mut self) {
        self.index = 0;
        self.highs.clear();
        self.lows.clear();
        self.last = None;
    }
}
