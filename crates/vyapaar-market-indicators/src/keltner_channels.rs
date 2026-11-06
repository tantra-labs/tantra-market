//! Keltner Channels.

use crate::atr::Atr;
use crate::ema::Ema;
use crate::indicator::Indicator;

/// The three Keltner lines.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct KeltnerValue {
    /// The EMA of the close.
    pub middle: f64,
    /// `middle + mult * ATR`.
    pub upper: f64,
    /// `middle − mult * ATR`.
    pub lower: f64,
}

/// Keltner Channels: an EMA of the close ± `mult` times the ATR of the same
/// period.
#[derive(Clone, Debug)]
pub struct KeltnerChannels {
    mult: f64,
    ema: Ema,
    atr: Atr,
    last: Option<KeltnerValue>,
}

impl KeltnerChannels {
    /// Creates Keltner Channels with the given EMA/ATR period and ATR
    /// multiplier.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero or `mult` is negative.
    pub fn new(period: usize, mult: f64) -> Self {
        assert!(period > 0, "Keltner period must be positive");
        assert!(mult >= 0.0, "Keltner mult must be non-negative");
        Self {
            mult,
            ema: Ema::new(period),
            atr: Atr::new(period),
            last: None,
        }
    }

    /// Returns the configured ATR multiplier.
    pub fn mult(&self) -> f64 {
        self.mult
    }

    /// Feeds one bar as raw `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: f64, low: f64, close: f64) -> Option<KeltnerValue> {
        // Both the EMA and the ATR must see every bar, even before either has
        // primed.
        let middle = self.ema.update(close);
        let atr = self.atr.update_hlc(high, low, close);
        let (m, a) = (middle?, atr?);
        let v = KeltnerValue {
            middle: m,
            upper: m + self.mult * a,
            lower: m - self.mult * a,
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for KeltnerChannels {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = KeltnerValue;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<KeltnerValue> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        let close = crate::bar_f64(&bar.close);
        self.update_hlc(high, low, close)
    }

    fn value(&self) -> Option<KeltnerValue> {
        self.last
    }

    fn reset(&mut self) {
        self.ema.reset();
        self.atr.reset();
        self.last = None;
    }
}
