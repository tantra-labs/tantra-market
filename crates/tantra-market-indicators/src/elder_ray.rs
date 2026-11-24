//! Elder Ray Index (Bull/Bear Power).

use rust_decimal::Decimal;

use crate::ema::Ema;
use crate::indicator::Indicator;

/// The two Elder Ray lines, computed in `Decimal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ElderRayValue {
    /// `high − EMA(period, close)`.
    pub bull_power: Decimal,
    /// `low − EMA(period, close)`.
    pub bear_power: Decimal,
}

/// Elder Ray Index, computed in `Decimal`.
///
/// Bull Power measures the buying pressure (`high − EMA`), Bear Power the
/// selling pressure (`low − EMA`), against the trend's EMA. The first value
/// appears once the EMA has primed (`period` bars).
#[derive(Clone, Debug)]
pub struct ElderRay {
    ema: Ema,
    last: Option<ElderRayValue>,
}

impl ElderRay {
    /// Creates an Elder Ray with the given EMA period.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "Elder Ray period must be positive");
        Self {
            ema: Ema::new(period),
            last: None,
        }
    }

    /// Returns the configured EMA period.
    pub fn period(&self) -> usize {
        self.ema.period()
    }

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`.
    pub fn update_hlc(
        &mut self,
        high: Decimal,
        low: Decimal,
        close: Decimal,
    ) -> Option<ElderRayValue> {
        let trend = self.ema.update(close)?;
        let v = ElderRayValue {
            bull_power: high - trend,
            bear_power: low - trend,
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for ElderRay {
    type Input<'a> = &'a tantra_core::Bar;
    type Output = ElderRayValue;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<ElderRayValue> {
        self.update_hlc(bar.high.value(), bar.low.value(), bar.close.value())
    }

    fn value(&self) -> Option<ElderRayValue> {
        self.last
    }

    fn reset(&mut self) {
        self.ema.reset();
        self.last = None;
    }
}
