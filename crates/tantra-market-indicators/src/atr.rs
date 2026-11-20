//! Average True Range (Wilder).

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tantra_core::Bar;

use crate::indicator::Indicator;

/// Wilder's Average True Range over `period` bars, computed in `Decimal`.
///
/// True range for a bar is:
///
/// `max(high - low, |high - prev_close|, |low - prev_close|)`
///
/// The first bar has no previous close, so its true range is just
/// `high - low`. The first ATR value is the SMA of the first `period` true
/// ranges; subsequent values use Wilder smoothing.
///
/// ```
/// use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
/// use tantra_market_indicators::{Atr, Indicator};
/// use rust_decimal::Decimal;
///
/// fn bar(h: i64, l: i64, c: i64) -> Bar {
///     Bar {
///         instrument: InstrumentId::new(Exchange::Nse, "X"),
///         open: Price::new(Decimal::from(c)),
///         high: Price::new(Decimal::from(h)),
///         low: Price::new(Decimal::from(l)),
///         close: Price::new(Decimal::from(c)),
///         volume: Quantity::from_u64(1),
///         timestamp: chrono::Utc::now(),
///     }
/// }
///
/// let mut atr = Atr::new(2);
/// assert_eq!(atr.update(&bar(10, 8, 9)), None);
/// assert_eq!(atr.update(&bar(11, 9, 10)), Some(Decimal::from(2)));
/// ```
#[derive(Clone, Debug)]
pub struct Atr {
    period: usize,
    prev_close: Option<Decimal>,
    seed_trs: Vec<Decimal>,
    atr: Option<Decimal>,
}

impl Atr {
    /// Creates an ATR over `period` bars.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "ATR period must be positive");
        Self {
            period,
            prev_close: None,
            seed_trs: Vec::with_capacity(period),
            atr: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Feeds one bar given as raw `high`, `low`, `close` and returns the ATR once warmed up.
    ///
    /// Identical to [`Indicator::update`] on a bar with those fields; it exists so callers that
    /// only hold price arrays need not build a `Bar`.
    pub fn update_hlc(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let tr = Self::true_range(high, low, self.prev_close);
        self.prev_close = Some(close);
        self.push_tr(tr)
    }

    /// `f64` presentation shim over [`Atr::update_hlc`].
    ///
    /// For chart/WASM presentation surfaces only; financial math must use
    /// [`Atr::update_hlc`]. Non-finite inputs return `None` without mutating state.
    pub fn update_hlc_f64(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        let h = Decimal::try_from(high).ok()?;
        let l = Decimal::try_from(low).ok()?;
        let c = Decimal::try_from(close).ok()?;
        self.update_hlc(h, l, c).and_then(|v| v.to_f64())
    }

    fn push_tr(&mut self, tr: Decimal) -> Option<Decimal> {
        let n = Decimal::from(self.period);
        match self.atr {
            None => {
                self.seed_trs.push(tr);
                if self.seed_trs.len() == self.period {
                    let seed = self.seed_trs.iter().copied().sum::<Decimal>() / n;
                    self.seed_trs.clear();
                    self.atr = Some(seed);
                    return Some(seed);
                }
                None
            }
            Some(prev) => {
                let next = (prev * (n - Decimal::ONE) + tr) / n;
                self.atr = Some(next);
                Some(next)
            }
        }
    }

    fn true_range(high: Decimal, low: Decimal, prev_close: Option<Decimal>) -> Decimal {
        let hl = high - low;
        match prev_close {
            None => hl,
            Some(pc) => hl.max((high - pc).abs()).max((low - pc).abs()),
        }
    }
}

impl Indicator for Atr {
    type Input<'a> = &'a Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &Bar) -> Option<Decimal> {
        self.update_hlc(bar.high.0, bar.low.0, bar.close.0)
    }

    fn value(&self) -> Option<Decimal> {
        self.atr
    }

    fn reset(&mut self) {
        self.prev_close = None;
        self.seed_trs.clear();
        self.atr = None;
    }
}
