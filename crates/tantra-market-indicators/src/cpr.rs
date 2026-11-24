//! Central Pivot Range (CPR).
//!
//! CPR is a popular intraday indicator in Indian markets that identifies key
//! support and resistance levels based on the previous day's OHLC.
//!
//! # Formula
//!
//! ```text
//! Pivot = (High + Low + Close) / 3
//! BC (Bottom Central Pivot) = (High + Low) / 2
//! TC (Top Central Pivot) = 2 * Pivot - BC
//! CPR Width = TC - BC
//! ```

use rust_decimal::Decimal;
use tantra_core::Bar;

use crate::indicator::Indicator;

/// Central Pivot Range (CPR) indicator, computed in `Decimal`.
///
/// CPR provides three key levels:
/// - Pivot: The central pivot point
/// - BC (Bottom Central Pivot): Lower boundary
/// - TC (Top Central Pivot): Upper boundary
#[derive(Clone, Debug)]
pub struct Cpr {
    pivot: Option<Decimal>,
    bc: Option<Decimal>,
    tc: Option<Decimal>,
    width: Option<Decimal>,
}

impl Cpr {
    /// Creates a new CPR calculator.
    pub fn new() -> Self {
        Self {
            pivot: None,
            bc: None,
            tc: None,
            width: None,
        }
    }

    /// Returns the current pivot value.
    pub fn pivot(&self) -> Option<Decimal> {
        self.pivot
    }

    /// Returns the bottom central pivot (BC).
    pub fn bc(&self) -> Option<Decimal> {
        self.bc
    }

    /// Returns the top central pivot (TC).
    pub fn tc(&self) -> Option<Decimal> {
        self.tc
    }

    /// Returns the CPR width (TC - BC).
    pub fn width(&self) -> Option<Decimal> {
        self.width
    }
}

impl Default for Cpr {
    fn default() -> Self {
        Self::new()
    }
}

impl Indicator for Cpr {
    type Input<'a> = &'a Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &Bar) -> Option<Decimal> {
        let high = bar.high.value();
        let low = bar.low.value();
        let close = bar.close.value();

        let three = Decimal::from(3);
        let pivot = (high + low + close) / three;
        let bc = (high + low) / Decimal::TWO;
        let tc = Decimal::TWO * pivot - bc;
        let width = tc - bc;

        self.pivot = Some(pivot);
        self.bc = Some(bc);
        self.tc = Some(tc);
        self.width = Some(width);

        Some(pivot)
    }

    fn value(&self) -> Option<Decimal> {
        self.pivot
    }

    fn reset(&mut self) {
        self.pivot = None;
        self.bc = None;
        self.tc = None;
        self.width = None;
    }
}
