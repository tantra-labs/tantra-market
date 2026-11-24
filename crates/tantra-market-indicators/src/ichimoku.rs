//! Ichimoku Cloud.

use std::collections::VecDeque;

use rust_decimal::Decimal;

use crate::indicator::Indicator;

/// The five Ichimoku lines as of the latest bar, computed in `Decimal`.
///
/// The 26-period *displacement* of the Senkou/Chikou spans is a charting
/// concern; a streaming indicator reports the current values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct IchimokuValue {
    /// Tenkan-sen: midpoint of the 9-bar high/low range.
    pub tenkan: Decimal,
    /// Kijun-sen: midpoint of the 26-bar high/low range.
    pub kijun: Decimal,
    /// Senkou Span A: `(tenkan + kijun) / 2`.
    pub senkou_a: Decimal,
    /// Senkou Span B: midpoint of the 52-bar high/low range.
    pub senkou_b: Decimal,
    /// Chikou Span: the latest close (the 26-bar back-shift is charting).
    pub chikou: Decimal,
}

/// Ichimoku Cloud (default 9/26/52 periods), computed in `Decimal`.
///
/// The first value appears after 52 bars (the Senkou B window).
#[derive(Clone, Debug)]
pub struct Ichimoku {
    tenkan_period: usize,
    kijun_period: usize,
    senkou_b_period: usize,
    highs: VecDeque<Decimal>,
    lows: VecDeque<Decimal>,
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
            last: None,
        }
    }

    /// Midpoint of the last `period` bars: `(max high + min low) / 2`.
    fn midpoint(highs: &VecDeque<Decimal>, lows: &VecDeque<Decimal>, period: usize) -> Decimal {
        let hi = highs
            .iter()
            .rev()
            .take(period)
            .copied()
            .max()
            .expect("non-empty window");
        let lo = lows
            .iter()
            .rev()
            .take(period)
            .copied()
            .min()
            .expect("non-empty window");
        (hi + lo) / Decimal::TWO
    }
}

impl Indicator for Ichimoku {
    type Input<'a> = &'a tantra_core::Bar;
    type Output = IchimokuValue;

    fn update(&mut self, bar: &tantra_core::Bar) -> Option<IchimokuValue> {
        let high = bar.high.value();
        let low = bar.low.value();
        let close = bar.close.value();
        self.highs.push_back(high);
        self.lows.push_back(low);
        if self.highs.len() > self.senkou_b_period {
            self.highs.pop_front();
            self.lows.pop_front();
        }
        if self.highs.len() < self.senkou_b_period {
            return None;
        }

        let tenkan = Self::midpoint(&self.highs, &self.lows, self.tenkan_period);
        let kijun = Self::midpoint(&self.highs, &self.lows, self.kijun_period);
        let senkou_a = (tenkan + kijun) / Decimal::TWO;
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
        self.last = None;
    }
}
