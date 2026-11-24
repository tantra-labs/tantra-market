//! ADX (Average Directional Index).
//!
//! ADX measures trend strength regardless of direction. It is derived from
//! +DI (Positive Directional Index) and -DI (Negative Directional Index).
//!
//! # Formula
//!
//! ```text
//! +DM = High - High[1] if High - High[1] > Low[1] - Low and High - High[1] > 0, else 0
//! -DM = Low[1] - Low if Low[1] - Low > High - High[1] and Low[1] - Low > 0, else 0
//! TR = max(High - Low, |High - Close[1]|, |Low - Close[1]|)
//! +DI = 100 * Smoothed(+DM) / Smoothed(TR)
//! -DI = 100 * Smoothed(-DM) / Smoothed(TR)
//! DX = 100 * |+DI - -DI| / (+DI + -DI)
//! ADX = Smoothed(DX)
//! ```

use rust_decimal::Decimal;
use tantra_core::Bar;

use crate::indicator::Indicator;

/// ADX (Average Directional Index) with +DI and -DI, computed in `Decimal`.
///
/// Uses Welles Wilder smoothing with O(1) state. The first ADX value appears
/// after `2 * period` bars (`1` baseline + `period` bars for initial +DI/-DI/DX
/// + `period - 1` additional bars to smooth the first `period` DX values).
#[derive(Clone, Debug)]
pub struct Adx {
    period: usize,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    prev_close: Option<Decimal>,
    dm_samples: usize,
    smoothed_tr: Decimal,
    smoothed_plus_dm: Decimal,
    smoothed_minus_dm: Decimal,
    dx_samples: usize,
    dx_sum: Decimal,
    last_plus_di: Option<Decimal>,
    last_minus_di: Option<Decimal>,
    last_dx: Option<Decimal>,
    last_adx: Option<Decimal>,
}

impl Adx {
    /// Creates a new ADX with the given period (typically 14).
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "ADX period must be positive");
        Self {
            period,
            prev_high: None,
            prev_low: None,
            prev_close: None,
            dm_samples: 0,
            smoothed_tr: Decimal::ZERO,
            smoothed_plus_dm: Decimal::ZERO,
            smoothed_minus_dm: Decimal::ZERO,
            dx_samples: 0,
            dx_sum: Decimal::ZERO,
            last_plus_di: None,
            last_minus_di: None,
            last_dx: None,
            last_adx: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Returns the current +DI value.
    pub fn plus_di(&self) -> Option<Decimal> {
        self.last_plus_di
    }

    /// Returns the current -DI value.
    pub fn minus_di(&self) -> Option<Decimal> {
        self.last_minus_di
    }

    /// Returns the current DX value.
    pub fn dx(&self) -> Option<Decimal> {
        self.last_dx
    }

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let (Some(prev_high), Some(prev_low), Some(prev_close)) =
            (self.prev_high, self.prev_low, self.prev_close)
        else {
            self.prev_high = Some(high);
            self.prev_low = Some(low);
            self.prev_close = Some(close);
            return None;
        };

        self.prev_high = Some(high);
        self.prev_low = Some(low);
        self.prev_close = Some(close);

        let up_move = high - prev_high;
        let down_move = prev_low - low;

        let plus_dm = if up_move > down_move && up_move > Decimal::ZERO {
            up_move
        } else {
            Decimal::ZERO
        };
        let minus_dm = if down_move > up_move && down_move > Decimal::ZERO {
            down_move
        } else {
            Decimal::ZERO
        };

        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());

        let n = Decimal::from(self.period);
        let hundred = Decimal::from(100);

        self.dm_samples += 1;
        if self.dm_samples <= self.period {
            self.smoothed_tr += tr;
            self.smoothed_plus_dm += plus_dm;
            self.smoothed_minus_dm += minus_dm;
            if self.dm_samples < self.period {
                return None;
            }
        } else {
            let decay = n - Decimal::ONE;
            self.smoothed_tr = self.smoothed_tr * decay / n + tr;
            self.smoothed_plus_dm = self.smoothed_plus_dm * decay / n + plus_dm;
            self.smoothed_minus_dm = self.smoothed_minus_dm * decay / n + minus_dm;
        }

        let (plus_di, minus_di) = if self.smoothed_tr.is_zero() {
            (Decimal::ZERO, Decimal::ZERO)
        } else {
            (
                hundred * self.smoothed_plus_dm / self.smoothed_tr,
                hundred * self.smoothed_minus_dm / self.smoothed_tr,
            )
        };
        let di_sum = plus_di + minus_di;
        let dx = if di_sum.is_zero() {
            Decimal::ZERO
        } else {
            hundred * (plus_di - minus_di).abs() / di_sum
        };

        self.last_plus_di = Some(plus_di);
        self.last_minus_di = Some(minus_di);
        self.last_dx = Some(dx);

        if let Some(prev_adx) = self.last_adx {
            let next_adx = (prev_adx * (n - Decimal::ONE) + dx) / n;
            self.last_adx = Some(next_adx);
            Some(next_adx)
        } else {
            self.dx_samples += 1;
            self.dx_sum += dx;
            if self.dx_samples == self.period {
                let first_adx = self.dx_sum / n;
                self.last_adx = Some(first_adx);
                Some(first_adx)
            } else {
                None
            }
        }
    }
}

impl Indicator for Adx {
    type Input<'a> = &'a Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &Bar) -> Option<Decimal> {
        self.update_hlc(bar.high.0, bar.low.0, bar.close.0)
    }

    fn value(&self) -> Option<Decimal> {
        self.last_adx
    }

    fn reset(&mut self) {
        self.prev_high = None;
        self.prev_low = None;
        self.prev_close = None;
        self.dm_samples = 0;
        self.smoothed_tr = Decimal::ZERO;
        self.smoothed_plus_dm = Decimal::ZERO;
        self.smoothed_minus_dm = Decimal::ZERO;
        self.dx_samples = 0;
        self.dx_sum = Decimal::ZERO;
        self.last_plus_di = None;
        self.last_minus_di = None;
        self.last_dx = None;
        self.last_adx = None;
    }
}
