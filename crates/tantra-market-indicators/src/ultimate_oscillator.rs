//! Ultimate Oscillator (Larry Williams).

use std::collections::VecDeque;

use rust_decimal::Decimal;

use crate::indicator::Indicator;

/// Ultimate Oscillator, computed in `Decimal`.
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
    prev_close: Option<Decimal>,
    bp: VecDeque<Decimal>,
    tr: VecDeque<Decimal>,
    last: Option<Decimal>,
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
    fn average(bp: &VecDeque<Decimal>, tr: &VecDeque<Decimal>, k: usize) -> Decimal {
        let start = bp.len().saturating_sub(k);
        let sum_bp: Decimal = bp.iter().skip(start).copied().sum();
        let sum_tr: Decimal = tr.iter().skip(start).copied().sum();
        if sum_tr.is_zero() {
            Decimal::ZERO
        } else {
            sum_bp / sum_tr
        }
    }

    /// Feeds one bar as raw `Decimal` `high`/`low`/`close`.
    pub fn update_hlc(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal> {
        let prev = match self.prev_close {
            None => {
                self.prev_close = Some(close);
                return None;
            }
            Some(p) => p,
        };
        self.prev_close = Some(close);

        let min_low = low.min(prev);
        let buying_pressure = close - min_low;
        let true_range = high.max(prev) - min_low;
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
        let four = Decimal::from(4);
        let seven = Decimal::from(7);
        let hundred = Decimal::from(100);
        let v = (four * avg7 + Decimal::TWO * avg14 + avg28) / seven * hundred;
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
    type Input<'a> = &'a tantra_core::Bar;
    type Output = Decimal;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<Decimal> {
        self.update_hlc(bar.high.value(), bar.low.value(), bar.close.value())
    }

    fn value(&self) -> Option<Decimal> {
        self.last
    }

    fn reset(&mut self) {
        self.prev_close = None;
        self.bp.clear();
        self.tr.clear();
        self.last = None;
    }
}
