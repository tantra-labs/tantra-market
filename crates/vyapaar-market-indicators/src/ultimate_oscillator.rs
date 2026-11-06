//! Ultimate Oscillator (Larry Williams).

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// Ultimate Oscillator.
///
/// `100 * (4*avg7 + 2*avg14 + avg28) / 7`, where
/// `avg_k = ΣBP / ΣTR` over the last `k` bars,
/// `BP = close − min(low, prev_close)` (buying pressure) and
/// `TR = max(high, prev_close) − min(low, prev_close)` (true range).
///
/// The three horizons damp single-bar noise; values above 70 / below 30 are
/// the classic overbought / oversold zones. The first value appears after 28
/// bars (the longest horizon).
#[derive(Clone, Debug)]
pub struct UltimateOscillator {
    prev_close: Option<f64>,
    bp: VecDeque<f64>,
    tr: VecDeque<f64>,
    last: Option<f64>,
}

impl UltimateOscillator {
    /// Creates an Ultimate Oscillator (fixed 7/14/28 horizons).
    pub fn new() -> Self {
        Self {
            prev_close: None,
            bp: VecDeque::with_capacity(28),
            tr: VecDeque::with_capacity(28),
            last: None,
        }
    }

    /// Average `ΣBP / ΣTR` over the last `k` entries (0 when TR is 0).
    fn average(bp: &VecDeque<f64>, tr: &VecDeque<f64>, k: usize) -> f64 {
        let start = bp.len().saturating_sub(k);
        let sum_bp: f64 = bp.iter().skip(start).sum();
        let sum_tr: f64 = tr.iter().skip(start).sum();
        if sum_tr == 0.0 { 0.0 } else { sum_bp / sum_tr }
    }

    /// Feeds one bar as raw `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        let prev = match self.prev_close {
            None => {
                self.prev_close = Some(close);
                return None;
            }
            Some(p) => p,
        };
        self.prev_close = Some(close);

        let buying_pressure = close - low.min(prev);
        let true_range = high.max(prev) - low.min(prev);
        self.bp.push_back(buying_pressure);
        self.tr.push_back(true_range);
        if self.bp.len() > 28 {
            self.bp.pop_front();
            self.tr.pop_front();
        }
        if self.bp.len() < 28 {
            return None;
        }

        let avg7 = Self::average(&self.bp, &self.tr, 7);
        let avg14 = Self::average(&self.bp, &self.tr, 14);
        let avg28 = Self::average(&self.bp, &self.tr, 28);
        let v = (4.0 * avg7 + 2.0 * avg14 + avg28) / 7.0 * 100.0;
        self.last = Some(v);
        Some(v)
    }
}

impl Default for UltimateOscillator {
    fn default() -> Self {
        Self::new()
    }
}

impl Indicator for UltimateOscillator {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = f64;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<f64> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        let close = crate::bar_f64(&bar.close);
        self.update_hlc(high, low, close)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.prev_close = None;
        self.bp.clear();
        self.tr.clear();
        self.last = None;
    }
}
