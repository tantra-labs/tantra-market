//! Order-block / elephant-bar zone detection.
//!
//! A bar is a displacement (elephant) candle when its range is at least
//! `displacement_atr_mult * ATR` and its body ratio
//! (`|close - open| / (high - low)`) is at least `min_body_ratio`. After a
//! displacement candle, the most recent preceding candle with the opposite
//! body direction is scanned back to, and its body range is emitted as the
//! order-block zone (`Support` for a bullish displacement, `Resistance` for a
//! bearish one). The zone is emitted once, on the displacement bar; ATR keeps
//! streaming. All math is exact `Decimal` (zero floats).

use std::collections::VecDeque;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tantra_core::Bar;
use tantra_core::levels::LevelSide;

use crate::indicator::Indicator;

/// A detected order-block zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderBlockValue {
    /// Zone side: `Support` for a bullish displacement (demand zone),
    /// `Resistance` for a bearish displacement (supply zone).
    pub side: LevelSide,
    /// Lower edge of the zone: the opposing candle's body low.
    pub low: Decimal,
    /// Upper edge of the zone: the opposing candle's body high.
    pub high: Decimal,
}

/// Direction of a candle body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyDir {
    Up,
    Down,
    Flat,
}

/// Order-block / elephant-bar detector (pure, zero floats, wasm-portable).
#[derive(Debug, Clone)]
pub struct OrderBlock {
    displacement_atr_mult: Decimal,
    min_body_ratio: Decimal,
    atr_period: usize,
    /// How many bars of body history to scan back for the opposing candle.
    lookback: usize,
    /// Simple Decimal ATR state (SMA seed then Wilder smoothing, like the
    /// crate's `Atr` but computed in `Decimal`).
    seed_trs: Vec<Decimal>,
    atr: Option<Decimal>,
    prev_close: Option<Decimal>,
    bars_seen: usize,
    /// `(dir, body_low, body_high)` of the most recent bars, oldest first.
    history: VecDeque<(BodyDir, Decimal, Decimal)>,
    value: Option<OrderBlockValue>,
}

impl OrderBlock {
    /// Creates an order-block detector.
    ///
    /// # Panics
    ///
    /// Panics if `atr_period` is zero.
    pub fn new(displacement_atr_mult: Decimal, min_body_ratio: Decimal, atr_period: u32) -> Self {
        assert!(atr_period > 0, "OrderBlock ATR period must be positive");
        let period = atr_period as usize;
        Self {
            displacement_atr_mult,
            min_body_ratio,
            atr_period: period,
            lookback: period,
            seed_trs: Vec::with_capacity(period),
            atr: None,
            prev_close: None,
            bars_seen: 0,
            history: VecDeque::with_capacity(period),
            value: None,
        }
    }

    fn body_dir(open: Decimal, close: Decimal) -> BodyDir {
        if close > open {
            BodyDir::Up
        } else if close < open {
            BodyDir::Down
        } else {
            BodyDir::Flat
        }
    }

    fn true_range(high: Decimal, low: Decimal, prev_close: Option<Decimal>) -> Decimal {
        let hl = high - low;
        match prev_close {
            None => hl,
            Some(pc) => {
                let hc = (high - pc).abs();
                let lc = (low - pc).abs();
                hl.max(hc).max(lc)
            }
        }
    }

    /// Pushes a true range and updates the Decimal ATR once seeded.
    fn push_tr(&mut self, tr: Decimal) {
        match self.atr {
            None => {
                self.seed_trs.push(tr);
                if self.seed_trs.len() == self.atr_period {
                    let n = Decimal::from(self.atr_period);
                    let sum: Decimal = self.seed_trs.iter().sum();
                    self.atr = Some(sum / n);
                }
            }
            Some(prev) => {
                let n = Decimal::from(self.atr_period);
                self.atr = Some((prev * (n - Decimal::ONE) + tr) / n);
            }
        }
    }

    /// Scans back for the most recent candle with a body direction opposite to
    /// `dir` and returns its body range.
    fn find_opposing_zone(&self, dir: BodyDir) -> Option<(Decimal, Decimal)> {
        let opposing = match dir {
            BodyDir::Up => BodyDir::Down,
            BodyDir::Down => BodyDir::Up,
            BodyDir::Flat => return None,
        };
        self.history
            .iter()
            .rev()
            .find_map(|(d, low, high)| (*d == opposing).then_some((*low, *high)))
    }
}

impl Indicator for OrderBlock {
    type Input<'a> = &'a Bar;
    type Output = OrderBlockValue;

    fn update(&mut self, bar: &Bar) -> Option<OrderBlockValue> {
        let open = bar.open.value();
        let high = bar.high.value();
        let low = bar.low.value();
        let close = bar.close.value();

        // Advance the Decimal ATR with the current bar, then evaluate.
        let tr = Self::true_range(high, low, self.prev_close);
        self.prev_close = Some(close);
        self.bars_seen += 1;
        self.push_tr(tr);

        // ATR warm-up: no output until `atr_period` bars have accumulated.
        if self.bars_seen < self.atr_period {
            return None;
        }
        let atr = self.atr?;

        let range = high - low;
        let body = (close - open).abs();
        let body_ratio = if range == Decimal::ZERO {
            Decimal::ZERO
        } else {
            body / range
        };
        let dir = Self::body_dir(open, close);

        let mut out = None;
        let displacement =
            range >= self.displacement_atr_mult * atr && body_ratio >= self.min_body_ratio;
        if displacement && let Some((zone_low, zone_high)) = self.find_opposing_zone(dir) {
            let side = match dir {
                BodyDir::Up => LevelSide::Support,
                BodyDir::Down => LevelSide::Resistance,
                BodyDir::Flat => LevelSide::Support,
            };
            let value = OrderBlockValue {
                side,
                low: zone_low,
                high: zone_high,
            };
            self.value = Some(value);
            out = Some(value);
        }

        self.history
            .push_back((dir, open.min(close), open.max(close)));
        if self.history.len() > self.lookback {
            self.history.pop_front();
        }
        if out.is_none() {
            self.value = None;
        }
        out
    }

    fn value(&self) -> Option<OrderBlockValue> {
        self.value
    }

    fn reset(&mut self) {
        self.seed_trs.clear();
        self.atr = None;
        self.prev_close = None;
        self.bars_seen = 0;
        self.history.clear();
        self.value = None;
    }
}
