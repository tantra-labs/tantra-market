//! Vortex Indicator (Etienne Botes / Douglas Siepman).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// The two Vortex lines.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct VortexValue {
    /// `Σ|H − prev_low| / ΣTR` over the window (positive-trend measure).
    pub vi_plus: f64,
    /// `Σ|L − prev_high| / ΣTR` over the window (negative-trend measure).
    pub vi_minus: f64,
}

/// Vortex Indicator.
///
/// True range here is `max(H − L, |H − prev_close|, |L − prev_close|)`. A
/// `vi_plus` crossing above `vi_minus` suggests an up-trend, and vice versa.
/// The first value appears after `period` bars.
#[derive(Clone, Debug)]
pub struct Vortex {
    period: usize,
    prev_close: Option<f64>,
    prev_high: Option<f64>,
    prev_low: Option<f64>,
    plus_vm: VecDeque<f64>,
    minus_vm: VecDeque<f64>,
    tr: VecDeque<f64>,
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

    /// Feeds one bar as raw `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: f64, low: f64, close: f64) -> Option<VortexValue> {
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

        let sum_tr: f64 = self.tr.iter().sum();
        let v = if sum_tr == 0.0 {
            VortexValue {
                vi_plus: 0.0,
                vi_minus: 0.0,
            }
        } else {
            VortexValue {
                vi_plus: self.plus_vm.iter().sum::<f64>() / sum_tr,
                vi_minus: self.minus_vm.iter().sum::<f64>() / sum_tr,
            }
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for Vortex {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = VortexValue;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<VortexValue> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        let close = crate::bar_f64(&bar.close);
        self.update_hlc(high, low, close)
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
