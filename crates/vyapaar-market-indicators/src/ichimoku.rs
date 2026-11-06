//! Ichimoku Cloud.

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// The five Ichimoku lines as of the latest bar.
///
/// The 26-period *displacement* of the Senkou/Chikou spans is a charting
/// concern; a streaming indicator reports the current values.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct IchimokuValue {
    /// Tenkan-sen: midpoint of the 9-bar high/low range.
    pub tenkan: f64,
    /// Kijun-sen: midpoint of the 26-bar high/low range.
    pub kijun: f64,
    /// Senkou Span A: `(tenkan + kijun) / 2`.
    pub senkou_a: f64,
    /// Senkou Span B: midpoint of the 52-bar high/low range.
    pub senkou_b: f64,
    /// Chikou Span: the latest close (the 26-bar back-shift is charting).
    pub chikou: f64,
}

/// Ichimoku Cloud (default 9/26/52 periods).
///
/// The first value appears after 52 bars (the Senkou B window).
#[derive(Clone, Debug)]
pub struct Ichimoku {
    tenkan_period: usize,
    kijun_period: usize,
    senkou_b_period: usize,
    highs: VecDeque<f64>,
    lows: VecDeque<f64>,
    closes: VecDeque<f64>,
    last: Option<IchimokuValue>,
}

impl Ichimoku {
    /// Creates an Ichimoku with the given Tenkan, Kijun and Senkou B periods.
    ///
    /// # Panics
    ///
    /// Panics if any period is zero or `kijun <= tenkan` or `senkou_b <= kijun`.
    pub fn new(tenkan_period: usize, kijun_period: usize, senkou_b_period: usize) -> Self {
        assert!(tenkan_period > 0, "Ichimoku Tenkan period must be positive");
        assert!(kijun_period > 0, "Ichimoku Kijun period must be positive");
        assert!(
            senkou_b_period > 0,
            "Ichimoku Senkou B period must be positive"
        );
        assert!(
            tenkan_period < kijun_period && kijun_period < senkou_b_period,
            "Ichimoku periods must satisfy tenkan < kijun < senkou_b"
        );
        Self {
            tenkan_period,
            kijun_period,
            senkou_b_period,
            highs: VecDeque::with_capacity(senkou_b_period),
            lows: VecDeque::with_capacity(senkou_b_period),
            closes: VecDeque::with_capacity(senkou_b_period),
            last: None,
        }
    }

    /// Midpoint of the last `period` bars: `(max high + min low) / 2`.
    fn midpoint(highs: &VecDeque<f64>, lows: &VecDeque<f64>, period: usize) -> f64 {
        let hi = highs
            .iter()
            .rev()
            .take(period)
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let lo = lows
            .iter()
            .rev()
            .take(period)
            .copied()
            .fold(f64::INFINITY, f64::min);
        (hi + lo) / 2.0
    }
}

impl Indicator for Ichimoku {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = IchimokuValue;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<IchimokuValue> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        let close = crate::bar_f64(&bar.close);
        self.highs.push_back(high);
        self.lows.push_back(low);
        self.closes.push_back(close);
        if self.highs.len() > self.senkou_b_period {
            self.highs.pop_front();
            self.lows.pop_front();
            self.closes.pop_front();
        }
        if self.highs.len() < self.senkou_b_period {
            return None;
        }

        let tenkan = Self::midpoint(&self.highs, &self.lows, self.tenkan_period);
        let kijun = Self::midpoint(&self.highs, &self.lows, self.kijun_period);
        let senkou_a = (tenkan + kijun) / 2.0;
        let senkou_b = Self::midpoint(&self.highs, &self.lows, self.senkou_b_period);

        let v = IchimokuValue {
            tenkan,
            kijun,
            senkou_a,
            senkou_b,
            chikou: close,
        };
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<IchimokuValue> {
        self.last
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.closes.clear();
        self.last = None;
    }
}
