//! Aroon Up/Down.

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// The two Aroon lines, in `[0, 100]`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct AroonValue {
    /// How recently the `period`-bar window hit its highest high.
    pub up: f64,
    /// How recently the `period`-bar window hit its lowest low.
    pub down: f64,
}

/// Aroon Up/Down.
///
/// `Aroon Up = 100 * (period − bars_since_highest_high) / period`, and
/// symmetrically for Down, over the last `period + 1` bars. A value near 100
/// means the extreme happened recently; near 0 means it happened long ago.
#[derive(Clone, Debug)]
pub struct Aroon {
    period: usize,
    highs: VecDeque<f64>,
    lows: VecDeque<f64>,
    last: Option<AroonValue>,
}

impl Aroon {
    /// Creates an Aroon over `period` bars.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "Aroon period must be positive");
        Self {
            period,
            highs: VecDeque::with_capacity(period + 1),
            lows: VecDeque::with_capacity(period + 1),
            last: None,
        }
    }

    /// Returns the configured period.
    pub fn period(&self) -> usize {
        self.period
    }

    /// Feeds one bar as raw `high`/`low`.
    pub fn update_hl(&mut self, high: f64, low: f64) -> Option<AroonValue> {
        self.highs.push_back(high);
        self.lows.push_back(low);
        if self.highs.len() > self.period + 1 {
            self.highs.pop_front();
            self.lows.pop_front();
        }
        if self.highs.len() < self.period + 1 {
            return None;
        }

        let n = self.highs.len();
        let max_high = self.highs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let min_low = self.lows.iter().copied().fold(f64::INFINITY, f64::min);

        // Bars since the extreme, counting from the most recent bar (index n-1)
        // backwards; the most recent occurrence wins ties.
        let mut since_high = self.period;
        let mut since_low = self.period;
        for i in (0..n).rev() {
            if since_high == self.period && self.highs[i] == max_high {
                since_high = n - 1 - i;
            }
            if since_low == self.period && self.lows[i] == min_low {
                since_low = n - 1 - i;
            }
            if since_high != self.period && since_low != self.period {
                break;
            }
        }

        let p = self.period as f64;
        let v = AroonValue {
            up: 100.0 * (p - since_high as f64) / p,
            down: 100.0 * (p - since_low as f64) / p,
        };
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for Aroon {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = AroonValue;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<AroonValue> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        self.update_hl(high, low)
    }

    fn value(&self) -> Option<AroonValue> {
        self.last
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.last = None;
    }
}
