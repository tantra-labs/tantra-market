//! Keltner Channels.

use rust_decimal::Decimal;

use crate::atr::Atr;
use crate::ema::Ema;
use crate::indicator::Indicator;

/// The three Keltner lines, computed in `Decimal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeltnerValue {
    /// The EMA of the close.
    pub middle: Decimal,
    /// `middle + mult * ATR`.
    pub upper: Decimal,
    /// `middle − mult * ATR`.
    pub lower: Decimal,
}

/// Keltner Channels: an EMA of the close ± `mult` times the ATR of the same
/// period, computed in `Decimal`.
#[derive(Clone, Debug)]
pub struct KeltnerChannels {
    mult: Decimal,
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
    pub fn new(period: usize, mult: Decimal) -> Self {
        assert!(period > 0, "Keltner period must be positive");
        assert!(mult >= Decimal::ZERO, "Keltner mult must be non-negative");
        Self {
            mult,
            ema: Ema::new(period),
            atr: Atr::new(period),
            last: None,
        }
    }

    /// Returns the configured ATR multiplier.
    pub fn mult(&self) -> Decimal {
        self.mult
    }

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`.
    pub fn update_hlc(
        &mut self,
        high: Decimal,
        low: Decimal,
        close: Decimal,
    ) -> Option<KeltnerValue> {
        // Both the EMA and the ATR must see every bar, even before either has
        // primed.
        let middle = self.ema.update(close);
        let atr = self.atr.update_hlc(high, low, close);
        let (m, a) = (middle?, atr?);
        let band = self.mult * a;
        let v = KeltnerValue {
            middle: m,
            upper: m + band,
            lower: m - band,
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for KeltnerChannels {
    type Input<'a> = &'a tantra_core::Bar;
    type Output = KeltnerValue;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<KeltnerValue> {
        self.update_hlc(bar.high.0, bar.low.0, bar.close.0)
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
