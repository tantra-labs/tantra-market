//! Primitive 1 & 3: Multi-asset SessionClock, SplitArtifactGuard & StandardIndicatorEvaluator.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;

use super::super::schedule::StudyAssetClass;
use super::types::{
    DetectedSplit, IndicatorPrimitive, IndicatorSet, IndicatorSpec, SessionPreset, SessionSlice,
    SplitCleanReport, StudyBar,
};
use crate::calendar::MarketCalendar;

/// NSE Sep 3, 2025 expiry rationalization cutover date.
const NSE_CUTOVER_YEAR: i32 = 2025;
const NSE_CUTOVER_MONTH: u32 = 9;
const NSE_CUTOVER_DAY: u32 = 3;

/// Primitive 1a: Multi-asset regime-aware session boundary slicer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionClock {
    preset: SessionPreset,
}

impl SessionClock {
    /// Creates a new [`SessionClock`] configured with `preset`.
    pub fn new(preset: SessionPreset) -> Self {
        Self { preset }
    }

    /// Returns the configured [`SessionPreset`].
    pub fn preset(&self) -> SessionPreset {
        self.preset
    }

    /// Slices chronological `bars` into contiguous [`SessionSlice`] regimes.
    pub fn slice_sessions(
        &self,
        bars: &[StudyBar],
        asset_class: StudyAssetClass,
        calendar: &dyn MarketCalendar,
    ) -> Vec<SessionSlice> {
        if bars.is_empty() {
            return Vec::new();
        }
        let cutover =
            NaiveDate::from_ymd_opt(NSE_CUTOVER_YEAR, NSE_CUTOVER_MONTH, NSE_CUTOVER_DAY).unwrap();

        let mut slices: Vec<SessionSlice> = Vec::new();
        for (idx, bar) in bars.iter().enumerate() {
            let (key, regime_tag) = self.classify_bar(bar, asset_class, calendar, cutover);
            if let Some(last) = slices.last_mut()
                && last.session_key == key
            {
                last.end_index = idx;
                continue;
            }
            slices.push(SessionSlice {
                session_key: key,
                start_index: idx,
                end_index: idx,
                session_open: bar.open,
                anchor_close: bar.close,
                regime_tag,
            });
        }
        slices
    }

    fn classify_bar(
        &self,
        bar: &StudyBar,
        asset_class: StudyAssetClass,
        calendar: &dyn MarketCalendar,
        cutover: NaiveDate,
    ) -> (String, String) {
        let d = bar.date;
        match self.preset {
            SessionPreset::NseSep2025Expiry => {
                let tag = if d >= cutover {
                    "Post-Cutover Tue-Expiry (NSE Sep 2025)"
                } else {
                    "Pre-Cutover Thu-Expiry (Legacy NSE)"
                };
                (d.format("%Y-%m-%d").to_string(), tag.to_string())
            }
            SessionPreset::NseEquityFno => {
                let holiday_tag = if calendar.is_holiday(d) {
                    "NSE Special Session"
                } else {
                    "NSE Regular (09:15-15:30 IST)"
                };
                (d.format("%Y-%m-%d").to_string(), holiday_tag.to_string())
            }
            SessionPreset::McxCommodity => {
                let session_type = match bar.minute_of_day {
                    Some(m) if m >= 1020 => "MCX Evening Session (09:00-23:30 IST)",
                    _ => "MCX Commodity Session (09:00-23:30 IST)",
                };
                (d.format("%Y-%m-%d").to_string(), session_type.to_string())
            }
            SessionPreset::FxSession => (
                d.format("%Y-%m-%d").to_string(),
                "FX 24h Session".to_string(),
            ),
            SessionPreset::DailyMonthly => {
                let key = format!("{:04}-{:02}", d.year(), d.month());
                let tag = match asset_class {
                    StudyAssetClass::Commodity => "Monthly Commodity Regime",
                    StudyAssetClass::Forex => "Monthly FX Regime",
                    _ => "Monthly Daily-Bar Regime",
                };
                (key, tag.to_string())
            }
            SessionPreset::WeeklyQuarter => {
                let q = ((d.month() - 1) / 3) + 1;
                (
                    format!("{:04}-Q{}", d.year(), q),
                    "Quarterly Weekly-Bar Regime".to_string(),
                )
            }
            SessionPreset::MonthlyYear => (
                format!("{:04}", d.year()),
                "Annual Monthly-Bar Regime".to_string(),
            ),
        }
    }
}

/// Primitive 1b: Corporate split & synthetic ATR-jump artifact scrubber.
///
/// Backward-adjusts historical bars strictly prior to any split bar (`split_factor > 1`)
/// or unannotated >1.95× overnight step discontinuity so ATR calculations never spike
/// across unadjusted corporate actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitArtifactGuard {
    /// Minimum overnight close-to-open ratio triggering synthetic split heuristic when `split_factor == 0`.
    pub unannotated_jump_threshold: Decimal,
}

impl Default for SplitArtifactGuard {
    fn default() -> Self {
        Self {
            // Only catches severe unannotated corporate actions (>= 3.5x drop / jump)
            unannotated_jump_threshold: Decimal::new(35, 1),
        }
    }
}

impl SplitArtifactGuard {
    /// Inspects `bars` for corporate splits and returns backward-adjusted continuous series.
    pub fn clean(&self, bars: &[StudyBar]) -> SplitCleanReport {
        if bars.is_empty() {
            return SplitCleanReport {
                adjusted_bars: Vec::new(),
                detected_splits: Vec::new(),
            };
        }

        let mut adjusted = bars.to_vec();
        let mut detected = Vec::new();
        let one = Decimal::ONE;

        for idx in 1..adjusted.len() {
            let explicit = adjusted[idx].split_factor;
            let (factor_opt, source) = if explicit > one {
                (Some(explicit), "explicit_split_factor")
            } else {
                let prev_close = adjusted[idx - 1].close;
                let cur_open = adjusted[idx].open;
                if prev_close > Decimal::ZERO && cur_open > Decimal::ZERO {
                    let ratio = prev_close / cur_open;
                    if ratio >= self.unannotated_jump_threshold {
                        // Round to nearest integer split multiple if close to integer (>= 2)
                        let rounded = ratio.round_dp(0);
                        if rounded >= Decimal::from(2)
                            && ((ratio - rounded).abs() / rounded) <= Decimal::new(20, 2)
                        {
                            (Some(rounded), "synthetic_gap_heuristic")
                        } else {
                            (None, "")
                        }
                    } else {
                        (None, "")
                    }
                } else {
                    (None, "")
                }
            };

            if let Some(factor) = factor_opt
                && factor > one
            {
                for prev in adjusted.iter_mut().take(idx) {
                    prev.open = (prev.open / factor).round_dp(2);
                    prev.high = (prev.high / factor).round_dp(2);
                    prev.low = (prev.low / factor).round_dp(2);
                    prev.close = (prev.close / factor).round_dp(2);
                    prev.volume = (prev.volume * factor).round_dp(0);
                }
                detected.push(DetectedSplit {
                    bar_index: idx,
                    date: adjusted[idx].date,
                    factor,
                    source: source.to_string(),
                });
            }
        }

        SplitCleanReport {
            adjusted_bars: adjusted,
            detected_splits: detected,
        }
    }
}

/// Primitive 3: Pure-`Decimal` standard indicator evaluator (`ATR`, `EMA`, `SMA`, `RSI`, `VWAP`, `VolumeRatio`).
#[derive(Clone, Copy, Debug, Default)]
pub struct StandardIndicatorEvaluator;

impl StandardIndicatorEvaluator {
    /// Computes Simple True Range ATR series (exact parity with TS benchmark `computeAtrSeries`).
    pub fn compute_atr_series(bars: &[StudyBar], period: usize) -> Vec<Decimal> {
        let p = period.max(1);
        let mut out = Vec::with_capacity(bars.len());
        let mut trs = Vec::with_capacity(bars.len());
        let mut sum = Decimal::ZERO;

        for (i, b) in bars.iter().enumerate() {
            let prev_close = if i > 0 { bars[i - 1].close } else { b.open };
            let hl = b.high - b.low;
            let hc = (b.high - prev_close).abs();
            let lc = (b.low - prev_close).abs();
            let tr = hl.max(hc).max(lc);
            trs.push(tr);
            sum += tr;
            if i >= p {
                sum -= trs[i - p];
                out.push((sum / Decimal::from(p as u64)).round_dp(4));
            } else {
                out.push((sum / Decimal::from((i + 1) as u64)).round_dp(4));
            }
        }
        out
    }

    fn compute_sma(bars: &[StudyBar], period: usize) -> Vec<Decimal> {
        let p = period.max(1);
        let mut out = Vec::with_capacity(bars.len());
        let mut sum = Decimal::ZERO;
        for (i, b) in bars.iter().enumerate() {
            sum += b.close;
            if i >= p {
                sum -= bars[i - p].close;
                out.push((sum / Decimal::from(p as u64)).round_dp(4));
            } else {
                out.push((sum / Decimal::from((i + 1) as u64)).round_dp(4));
            }
        }
        out
    }

    fn compute_ema(bars: &[StudyBar], period: usize) -> Vec<Decimal> {
        if bars.is_empty() {
            return Vec::new();
        }
        let p = Decimal::from(period.max(1) as u64);
        let two = Decimal::from(2);
        let alpha = two / (p + Decimal::ONE);
        let one_minus = Decimal::ONE - alpha;
        let mut out = Vec::with_capacity(bars.len());
        let mut ema = bars[0].close;
        out.push(ema.round_dp(4));
        for b in bars.iter().skip(1) {
            ema = (alpha * b.close + one_minus * ema).round_dp(6);
            out.push(ema.round_dp(4));
        }
        out
    }

    fn compute_rsi(bars: &[StudyBar], period: usize) -> Vec<Decimal> {
        if bars.is_empty() {
            return Vec::new();
        }
        let p = period.max(2);
        let mut out = Vec::with_capacity(bars.len());
        let fifty = Decimal::from(50);
        let hundred = Decimal::from(100);
        out.push(fifty);

        let mut gains = Vec::with_capacity(bars.len());
        let mut losses = Vec::with_capacity(bars.len());
        gains.push(Decimal::ZERO);
        losses.push(Decimal::ZERO);

        let mut gain_sum = Decimal::ZERO;
        let mut loss_sum = Decimal::ZERO;

        for i in 1..bars.len() {
            let diff = bars[i].close - bars[i - 1].close;
            let g = if diff > Decimal::ZERO {
                diff
            } else {
                Decimal::ZERO
            };
            let l = if diff < Decimal::ZERO {
                diff.abs()
            } else {
                Decimal::ZERO
            };
            gains.push(g);
            losses.push(l);
            gain_sum += g;
            loss_sum += l;
            if i > p {
                gain_sum -= gains[i - p];
                loss_sum -= losses[i - p];
            }
            let denom_n = Decimal::from(i.min(p) as u64);
            let avg_g = gain_sum / denom_n;
            let avg_l = loss_sum / denom_n;
            let rsi = if avg_l.is_zero() {
                if avg_g.is_zero() { fifty } else { hundred }
            } else {
                let rs = avg_g / avg_l;
                hundred - (hundred / (Decimal::ONE + rs))
            };
            out.push(rsi.round_dp(2));
        }
        out
    }

    fn compute_vwap(bars: &[StudyBar]) -> Vec<Decimal> {
        let mut out = Vec::with_capacity(bars.len());
        let three = Decimal::from(3);
        let mut cum_pv = Decimal::ZERO;
        let mut cum_v = Decimal::ZERO;
        for b in bars {
            let tp = (b.high + b.low + b.close) / three;
            let v = if b.volume > Decimal::ZERO {
                b.volume
            } else {
                Decimal::ONE
            };
            cum_pv += tp * v;
            cum_v += v;
            if cum_v > Decimal::ZERO {
                out.push((cum_pv / cum_v).round_dp(4));
            } else {
                out.push(tp.round_dp(4));
            }
        }
        out
    }

    fn compute_volume_ratio(bars: &[StudyBar], period: usize) -> Vec<Decimal> {
        let p = period.max(1);
        let mut out = Vec::with_capacity(bars.len());
        let mut vol_sum = Decimal::ZERO;
        for (i, b) in bars.iter().enumerate() {
            vol_sum += b.volume;
            if i >= p {
                vol_sum -= bars[i - p].volume;
            }
            let n = Decimal::from((i + 1).min(p) as u64);
            let avg = vol_sum / n;
            let ratio = if avg > Decimal::ZERO {
                (b.volume / avg).round_dp(4)
            } else {
                Decimal::ONE
            };
            out.push(ratio);
        }
        out
    }
}

impl IndicatorPrimitive for StandardIndicatorEvaluator {
    fn evaluate(&self, bars: &[StudyBar], specs: &[IndicatorSpec]) -> IndicatorSet {
        let mut set = IndicatorSet {
            atr: Self::compute_atr_series(bars, 14),
            ema: BTreeMap::new(),
            sma: BTreeMap::new(),
            rsi: Vec::new(),
            vwap: Vec::new(),
            volume_ratio: Vec::new(),
        };
        for spec in specs {
            match *spec {
                IndicatorSpec::Atr { period } => {
                    set.atr = Self::compute_atr_series(bars, period);
                }
                IndicatorSpec::Ema { period } => {
                    set.ema.insert(period, Self::compute_ema(bars, period));
                }
                IndicatorSpec::Sma { period } => {
                    set.sma.insert(period, Self::compute_sma(bars, period));
                }
                IndicatorSpec::Rsi { period } => {
                    set.rsi = Self::compute_rsi(bars, period);
                }
                IndicatorSpec::Vwap => {
                    set.vwap = Self::compute_vwap(bars);
                }
                IndicatorSpec::VolumeRatio { period } => {
                    set.volume_ratio = Self::compute_volume_ratio(bars, period);
                }
            }
        }
        set
    }
}
