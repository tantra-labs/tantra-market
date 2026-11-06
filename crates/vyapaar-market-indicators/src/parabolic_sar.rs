//! Parabolic SAR (Welles Wilder).

use crate::indicator::Indicator;

/// Parabolic SAR.
///
/// Starts in an uptrend on the second bar with `SAR = first low`,
/// `EP = highest high so far` and an acceleration factor (`AF`) of 0.02. Each
/// bar advances `SAR += AF * (EP − SAR)` (clamped inside the prior low for
/// uptrends / prior high for downtrends); a new extreme bumps `EP` and adds
/// 0.02 to `AF` up to 0.20. The trend flips when price crosses SAR.
#[derive(Clone, Debug)]
pub struct ParabolicSar {
    af_step: f64,
    af_max: f64,
    sar: Option<f64>,
    af: f64,
    ep: f64,
    is_up: bool,
    prev_high: Option<f64>,
    prev_low: Option<f64>,
    last: Option<f64>,
}

impl Default for ParabolicSar {
    fn default() -> Self {
        Self::new(0.02, 0.20)
    }
}

impl ParabolicSar {
    /// Creates a Parabolic SAR with the given acceleration step and cap.
    ///
    /// # Panics
    ///
    /// Panics if `af_step` is non-positive or `af_max < af_step`.
    pub fn new(af_step: f64, af_max: f64) -> Self {
        assert!(af_step > 0.0, "SAR acceleration step must be positive");
        assert!(af_max >= af_step, "SAR acceleration cap must be >= step");
        Self {
            af_step,
            af_max,
            sar: None,
            af: af_step,
            ep: 0.0,
            is_up: true,
            prev_high: None,
            prev_low: None,
            last: None,
        }
    }

    /// Feeds one bar as raw `high`/`low` and returns the SAR from the second
    /// bar onwards.
    pub fn update_hl(&mut self, high: f64, low: f64) -> Option<f64> {
        match (self.prev_high, self.prev_low) {
            // First bar: nothing to hang a SAR on yet.
            (None, None) => {
                self.prev_high = Some(high);
                self.prev_low = Some(low);
                return None;
            }
            (Some(prev_high), Some(prev_low)) => {
                if self.sar.is_none() {
                    // Second bar: seed an uptrend on the first bar's low.
                    self.sar = Some(prev_low);
                    self.af = self.af_step;
                    self.ep = high.max(prev_high);
                    self.is_up = true;
                } else {
                    let prev_sar = self.sar.unwrap();
                    // Next SAR (unclamped) — the trend-flip test is against
                    // this value, exactly as in the classic algorithm.
                    let next_sar = prev_sar + self.af * (self.ep - prev_sar);

                    if self.is_up && low < next_sar {
                        // Flip to a downtrend.
                        self.is_up = false;
                        self.sar = Some(self.ep);
                        self.af = self.af_step;
                        self.ep = low;
                    } else if !self.is_up && high > next_sar {
                        // Flip to an uptrend.
                        self.is_up = true;
                        self.sar = Some(self.ep);
                        self.af = self.af_step;
                        self.ep = high;
                    } else {
                        // Stay in trend; clamp the SAR inside the prior/current
                        // extreme so it cannot penetrate price.
                        let clamped = if self.is_up {
                            next_sar.min(prev_low.min(low))
                        } else {
                            next_sar.max(prev_high.max(high))
                        };
                        self.sar = Some(clamped);
                        if self.is_up {
                            if high > self.ep {
                                self.ep = high;
                                self.af = (self.af + self.af_step).min(self.af_max);
                            }
                        } else if low < self.ep {
                            self.ep = low;
                            self.af = (self.af + self.af_step).min(self.af_max);
                        }
                    }
                }
            }
            _ => unreachable!("prev_high/prev_low are set together"),
        }
        self.prev_high = Some(high);
        self.prev_low = Some(low);
        let v = self.sar.unwrap();
        self.last = Some(v);
        Some(v)
    }
}

impl Indicator for ParabolicSar {
    type Input<'a> = &'a vyapaar_core::Bar;
    type Output = f64;

    fn update(&mut self, bar: &vyapaar_core::Bar) -> Option<f64> {
        let high = crate::bar_f64(&bar.high);
        let low = crate::bar_f64(&bar.low);
        self.update_hl(high, low)
    }

    fn value(&self) -> Option<f64> {
        self.last
    }

    fn reset(&mut self) {
        self.sar = None;
        self.af = self.af_step;
        self.ep = 0.0;
        self.is_up = true;
        self.prev_high = None;
        self.prev_low = None;
        self.last = None;
    }
}
