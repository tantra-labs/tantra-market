//! OBV (On Balance Volume).
//!
//! OBV is a momentum indicator that uses volume flow to predict changes in
//! stock price. It adds volume on up days and subtracts volume on down days.
//!
//! # Formula
//!
//! ```text
//! OBV = OBV[1] + Volume if Close > Close[1]
//! OBV = OBV[1] - Volume if Close < Close[1]
//! OBV = OBV[1] if Close == Close[1]
//! ```

use rust_decimal::Decimal;
use tantra_core::Bar;

use crate::indicator::Indicator;

/// On Balance Volume (OBV), computed in `Decimal`.
#[derive(Clone, Debug)]
pub struct Obv {
    value: Decimal,
    prev_close: Option<Decimal>,
}

impl Obv {
    /// Creates a new OBV calculator.
    pub fn new() -> Self {
        Self {
            value: Decimal::ZERO,
            prev_close: None,
        }
    }

    /// Returns the current OBV value.
    pub fn value(&self) -> Option<Decimal> {
        Some(self.value)
    }
}

impl Default for Obv {
    fn default() -> Self {
        Self::new()
    }
}

impl Indicator for Obv {
    type Input<'a> = &'a Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &Bar) -> Option<Decimal> {
        let close = bar.close.0;
        let volume = bar.volume.0;

        if let Some(prev_close) = self.prev_close {
            if close > prev_close {
                self.value += volume;
            } else if close < prev_close {
                self.value -= volume;
            }
        }
        self.prev_close = Some(close);

        Some(self.value)
    }

    fn value(&self) -> Option<Decimal> {
        Some(self.value)
    }

    fn reset(&mut self) {
        self.value = Decimal::ZERO;
        self.prev_close = None;
    }
}
