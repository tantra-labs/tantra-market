//! Vortex Indicator (Etienne Botes / Douglas Siepman).

use std::collections::VecDeque;

use rust_decimal::Decimal;

use crate::indicator::Indicator;

/// The two Vortex lines, computed in `Decimal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VortexValue {
    /// `Σ|H − prev_low| / ΣTR` over the window (positive-trend measure).
    pub vi_plus: Decimal,
    /// `Σ|L − prev_high| / ΣTR` over the window (negative-trend measure).
    pub vi_minus: Decimal,
}

/// Vortex Indicator, computed in `Decimal`.
///
/// True range here is `max(H − L, |H − prev_close|, |L − prev_close|)`. A
/// `vi_plus` crossing above `vi_minus` suggests an up-trend, and vice versa.
/// The first value appears after `period` bars.
#[derive(Clone, Debug)]
pub struct Vortex {
    period: usize,
    prev_close: Option<Decimal>,
    prev_high: Option<Decimal>,
    prev_low: Option<Decimal>,
    plus_vm: VecDeque<Decimal>,
    minus_vm: VecDeque<Decimal>,
    tr: VecDeque<Decimal>,
    last: Option<VortexValue>,
}

impl Vortex {
    /// Creates a Vortex over `period` bars.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "Vortex period must be positive");
        Self {
            period,
            prev_close: None,
            prev_high: None,
            prev_low: None,
            plus_vm: VecDeque::with_capacity(period),
            minus_vm: VecDeque::with_capacity(period),
            tr: VecDeque::with_capacity(period),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`.
    pub fn update_hlc(
        &mut self,
        high: Decimal,
        low: Decimal,
        close: Decimal,
    ) -> Option<VortexValue> {
        let prev_close = match self.prev_close {
            None => {
                self.prev_close = Some(close);
                self.prev_high = Some(high);
                self.prev_low = Some(low);
                return None;
            }
            Some(pc) => pc,
        };
        let prev_high = self.prev_high.unwrap();
        let prev_low = self.prev_low.unwrap();

        let true_range = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        let plus = (high - prev_low).abs();
        let minus = (low - prev_high).abs();

        self.prev_close = Some(close);
        self.prev_high = Some(high);
        self.prev_low = Some(low);
        self.plus_vm.push_back(plus);
        self.minus_vm.push_back(minus);
        self.tr.push_back(true_range);
        if self.plus_vm.len() > self.period {
            self.plus_vm.pop_front();
            self.minus_vm.pop_front();
            self.tr.pop_front();
        }
        if self.plus_vm.len() < self.period {
            return None;
        }

        let sum_tr: Decimal = self.tr.iter().copied().sum();
        let v = if sum_tr.is_zero() {
            VortexValue {
                vi_plus: Decimal::ZERO,
                vi_minus: Decimal::ZERO,
            }
        } else {
            VortexValue {
                vi_plus: self.plus_vm.iter().copied().sum::<Decimal>() / sum_tr,
                vi_minus: self.minus_vm.iter().copied().sum::<Decimal>() / sum_tr,
            }
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for Vortex {
    type Input<'a> = &'a tantra_core::Bar;
    type Output = VortexValue;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<VortexValue> {
        self.update_hlc(bar.high.value(), bar.low.value(), bar.close.value())
    }

    fn value(&self) -> Option<VortexValue> {
        self.last
    }

    fn reset(&mut self) {
        self.prev_close = None;
        self.prev_high = None;
        self.prev_low = None;
        self.plus_vm.clear();
        self.minus_vm.clear();
        self.tr.clear();
        self.last = None;
    }
}
