//! Supertrend indicator.
//!
//! Supertrend is an ATR-based trend-following indicator that provides dynamic
//! support/resistance bands. It flips direction when price crosses the band,
//! making it ideal for trending markets.
//!
//! # Formula
//!
//! ```text
//! HL2 = (High + Low) / 2
//! Basic Upper Band = HL2 + (Multiplier * ATR)
//! Basic Lower Band = HL2 - (Multiplier * ATR)
//! Final Upper Band = Basic Upper Band if Basic Upper Band < Prev Final Upper Band or Close[1] > Prev Final Upper Band else Prev Final Upper Band
//! Final Lower Band = Basic Lower Band if Basic Lower Band > Prev Final Lower Band or Close[1] < Prev Final Lower Band else Prev Final Lower Band
//! Supertrend = Final Lower Band in uptrend, Final Upper Band in downtrend
//! ```

use rust_decimal::Decimal;
use tantra_core::Bar;

use crate::atr::Atr;
use crate::indicator::Indicator;

/// Supertrend indicator, computed in `Decimal` with O(1) state.
///
/// Parameters:
/// - `period`: ATR period (default: 10)
/// - `multiplier`: ATR multiplier for band width (default: 3)
#[derive(Clone, Debug)]
pub struct Supertrend {
    period: usize,
    multiplier: Decimal,
    atr: Atr,
    prev_close: Option<Decimal>,
    prev_upper: Option<Decimal>,
    prev_lower: Option<Decimal>,
    in_uptrend: bool,
    last: Option<Decimal>,
}

impl Supertrend {
    /// Creates a new Supertrend with the given period and multiplier.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero or `multiplier` is negative.
    pub fn new(period: usize, multiplier: Decimal) -> Self {
        assert!(period > 0, "Supertrend period must be positive");
        assert!(
            multiplier >= Decimal::ZERO,
            "Supertrend multiplier must be non-negative"
        );
        Self {
            period,
            multiplier,
            atr: Atr::new(period),
            prev_close: None,
            prev_upper: None,
            prev_lower: None,
            in_uptrend: true,
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Returns the configured multiplier.
    pub fn multiplier(&self) -> Decimal {
        self.multiplier
    }

    /// Returns true if currently in uptrend.
    pub fn is_uptrend(&self) -> bool {
        self.in_uptrend
    }

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let atr_val = self.atr.update_hlc(high, low, close);
        let prior_close = self.prev_close;
        self.prev_close = Some(close);

        let atr = atr_val?;
        let hl2 = (high + low) / Decimal::TWO;
        let band = self.multiplier * atr;
        let basic_upper = hl2 + band;
        let basic_lower = hl2 - band;

        let final_upper = match (self.prev_upper, prior_close) {
            (Some(pu), Some(pc)) if basic_upper >= pu && pc <= pu => pu,
            _ => basic_upper,
        };
        let final_lower = match (self.prev_lower, prior_close) {
            (Some(pl), Some(pc)) if basic_lower <= pl && pc >= pl => pl,
            _ => basic_lower,
        };

        let (st, is_up) = match self.last {
            None => {
                if close >= final_lower {
                    (final_lower, true)
                } else {
                    (final_upper, false)
                }
            }
            Some(_) if !self.in_uptrend => {
                if close <= final_upper {
                    (final_upper, false)
                } else {
                    (final_lower, true)
                }
            }
            Some(_) => {
                if close >= final_lower {
                    (final_lower, true)
                } else {
                    (final_upper, false)
                }
            }
        };

        self.prev_upper = Some(final_upper);
        self.prev_lower = Some(final_lower);
        self.in_uptrend = is_up;
        self.last = Some(st);
        Some(st)
    }
}

impl Indicator for Supertrend {
    type Input<'a> = &'a Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &Bar) -> Option<Decimal> {
        self.update_hlc(bar.high.0, bar.low.0, bar.close.0)
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.atr.reset();
        self.prev_close = None;
        self.prev_upper = None;
        self.prev_lower = None;
        self.in_uptrend = true;
        self.last = None;
    }
}
