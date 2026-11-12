//! Know Sure Thing (KST) — Martin Pring.

use std::collections::VecDeque;

use crate::indicator::Indicator;

/// One ROC → SMA pipeline (a smoothed rate of change, "RCMA").
#[derive(Clone, Debug)]
struct RocSmaPipeline {
    roc_period: usize,
    sma_period: usize,
    prices: VecDeque<f64>,
    rocs: VecDeque<f64>,
}

impl RocSmaPipeline {
    fn new(roc_period: usize, sma_period: usize) -> Self {
        Self {
            roc_period,
            sma_period,
            prices: VecDeque::with_capacity(roc_period + 1),
            rocs: VecDeque::with_capacity(sma_period),
        }
    }

    /// Feeds one close and returns the current RCMA once enough history has
    /// accumulated (a zero price `roc_period` bars ago yields no ROC).
    fn update(&mut self, price: f64) -> Option<f64> {
        self.prices.push_back(price);
        if self.prices.len() > self.roc_period + 1 {
            self.prices.pop_front();
        }
        if self.prices.len() == self.roc_period + 1 {
            let oldest = self.prices.front().copied().unwrap_or(0.0);
            if oldest != 0.0 {
                let roc = (price - oldest) / oldest * 100.0;
                self.rocs.push_back(roc);
                if self.rocs.len() > self.sma_period {
                    self.rocs.pop_front();
                }
            }
        }
        if self.rocs.len() == self.sma_period {
            Some(self.rocs.iter().sum::<f64>() / self.sma_period as f64)
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.prices.clear();
        self.rocs.clear();
    }
}

/// Know Sure Thing (KST).
///
/// `KST = RCMA1 + 2·RCMA2 + 3·RCMA3 + 4·RCMA4` with Pring's standard pipeline:
///
/// | leg | ROC period | smoothing |
/// |-----|-----------|-----------|
/// | 1   | 10        | SMA 10    |
/// | 2   | 15        | SMA 10    |
/// | 3   | 20        | SMA 10    |
/// | 4   | 30        | SMA 15    |
///
/// The first value appears after `30 + 15 = 45` closes: the ROC(30) leg needs
/// 31 closes for its first rate of change, and its SMA(15) then needs 15 of
/// them.
#[derive(Clone, Debug)]
pub struct Kst {
    p1: RocSmaPipeline,
    p2: RocSmaPipeline,
    p3: RocSmaPipeline,
    p4: RocSmaPipeline,
    last: Option<f64>,
}

impl Default for Kst {
    fn default() -> Self {
        Self::new()
    }
}

impl Kst {
    /// Creates a KST with Pring's standard 10/15/20/30 ROC and 10/10/10/15
    /// smoothing periods.
    pub fn new() -> Self {
        Self {
            p1: RocSmaPipeline::new(10, 10),
            p2: RocSmaPipeline::new(15, 10),
            p3: RocSmaPipeline::new(20, 10),
            p4: RocSmaPipeline::new(30, 15),
            last: None,
        }
    }
}

impl Indicator for Kst {
    type Input<'a> = f64;
    type Output = f64;

    fn update(&mut self, price: f64) -> Option<f64> {
        // Every pipeline must see every close regardless of readiness.
        let rcma1 = self.p1.update(price);
        let rcma2 = self.p2.update(price);
        let rcma3 = self.p3.update(price);
        let rcma4 = self.p4.update(price);
        let (r1, r2, r3, r4) = (rcma1?, rcma2?, rcma3?, rcma4?);
        let v = r1 + 2.0 * r2 + 3.0 * r3 + 4.0 * r4;
        self.last = Some(v);
        Some(v)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.p1.reset();
        self.p2.reset();
        self.p3.reset();
        self.p4.reset();
        self.last = None;
    }
}
