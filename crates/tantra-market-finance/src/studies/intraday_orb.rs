//! Pure `Decimal` Intraday Movement, Gap, 5-Day Move, Cycle Progress (0-100%)
//! and 15m ORB Flip Backtest studies (`wasm32`-ready).

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::decimal_sqrt;
use super::distributions::empirical_quantile;
use super::returns::mean_and_sample_std;
use super::schedule::{OhlcvBar, SlicedPeriod, StudyAssetClass};
use super::spreads::estimate_statutory_roundtrip_bps;

/// Single intraday bar (`5m` or `15m`) stamped in exchange local calendar time (e.g. IST).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntradayBar {
    /// Local exchange calendar date.
    pub date: NaiveDate,
    /// Minutes since `00:00` local exchange time (`0..1440`, e.g. `555` = `09:15`).
    pub minute_of_day: u16,
    /// Bar open price.
    pub open: Decimal,
    /// Bar high price.
    pub high: Decimal,
    /// Bar low price.
    pub low: Decimal,
    /// Bar close price.
    pub close: Decimal,
    /// Bar volume.
    pub volume: Decimal,
}

/// Statistics for one time-of-day intraday bucket (`15m` or `60m`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntradaySlotStat {
    /// Human-readable clock slot (`HH:MM`, e.g. `"09:15"`).
    pub slot_label: String,
    /// Minute offset used for sorting.
    pub minute_of_day: u16,
    /// Number of intraday bars observed in this clock slot.
    pub bar_count: usize,
    /// Mean intra-bar range % `(high - low) / open * 100`.
    pub avg_range_pct: Decimal,
    /// Maximum observed range % in this slot.
    pub max_range_pct: Decimal,
    /// Mean absolute close-to-open move % `|close - open| / open * 100`.
    pub avg_abs_move_pct: Decimal,
    /// Average bar volume.
    pub avg_volume: Decimal,
    /// Number of sessions whose High-of-Day occurred inside this slot.
    pub hod_count: usize,
    /// Number of sessions whose Low-of-Day occurred inside this slot.
    pub lod_count: usize,
}

/// Tab 3: Intraday movement, session range & HOD/LOD clock profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IntradayMovementAnalysis {
    /// Whether `5m`/intraday bars were available for this instrument.
    pub available: bool,
    /// Number of distinct trading sessions with intraday bars.
    pub total_sessions: usize,
    /// Average session high-low range (%).
    pub avg_session_range_pct: Decimal,
    /// 15-minute slot with peak average volatility.
    pub peak_volatility_slot: String,
    /// Most frequent High-of-Day clock slot.
    pub most_common_hod_slot: String,
    /// Most frequent Low-of-Day clock slot.
    pub most_common_lod_slot: String,
    /// Breakdown across 15-minute buckets.
    pub slots_15m: Vec<IntradaySlotStat>,
    /// Breakdown across 60-minute hourly buckets.
    pub slots_hourly: Vec<IntradaySlotStat>,
}

/// One bucket row in the Overnight Gap distribution table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapBucketRow {
    /// Display bucket label (e.g. `"+0.5% to +1.0%"`).
    pub label: String,
    /// Observed count.
    pub count: usize,
    /// Share of all gap sessions (%).
    pub share_pct: Decimal,
    /// Gap-fill rate (%) inside this bucket.
    pub fill_rate_pct: Decimal,
    /// Mean intraday open-to-close move (%) following gaps in this bucket.
    pub avg_intraday_follow_pct: Decimal,
}

/// Individual notable overnight gap record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapEventRow {
    /// Session date.
    pub date: NaiveDate,
    /// Prior close.
    pub prev_close: Decimal,
    /// Current open.
    pub open: Decimal,
    /// Current close.
    pub close: Decimal,
    /// Overnight gap %.
    pub gap_pct: Decimal,
    /// Intraday open-to-close return %.
    pub intraday_return_pct: Decimal,
    /// Whether the session touched `prev_close` (gap filled).
    pub filled: bool,
}

/// Tab 4: Overnight gap distribution, fill probability & continuation expectancy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GapAnalysis {
    /// Total sessions evaluated (`daily_bars.len() - 1`).
    pub total_sessions: usize,
    /// Count of gap-up sessions (`gap_pct > 0`).
    pub gap_up_count: usize,
    /// Count of gap-down sessions (`gap_pct < 0`).
    pub gap_down_count: usize,
    /// Average positive gap magnitude (%).
    pub avg_gap_up_pct: Decimal,
    /// Average negative gap magnitude (%).
    pub avg_gap_down_pct: Decimal,
    /// Overall gap-up fill probability (%).
    pub gap_up_fill_rate_pct: Decimal,
    /// Overall gap-down fill probability (%).
    pub gap_down_fill_rate_pct: Decimal,
    /// Average open-to-close fade/continuation after a gap up (%).
    pub gap_up_follow_pct: Decimal,
    /// Average open-to-close fade/continuation after a gap down (%).
    pub gap_down_follow_pct: Decimal,
    /// Bucketed distribution + conditional fill & follow-through rates.
    pub buckets: Vec<GapBucketRow>,
    /// Most recent / largest gap events (capped to 40 rows).
    pub recent_events: Vec<GapEventRow>,
}

/// One point in the Rolling 5-Day Move trajectory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollingFiveDayPoint {
    /// Session end date.
    pub date: NaiveDate,
    /// Session close.
    pub close: Decimal,
    /// Trailing 5-trading-day return (%).
    pub return_5d_pct: Decimal,
}

/// Tab 5: Rolling 5-trading-day move distribution & exhaustion profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RollingFiveDayAnalysis {
    /// Total 5-day windows evaluated.
    pub sample_count: usize,
    /// Mean 5-day return (%).
    pub avg_return_pct: Decimal,
    /// Sample standard deviation of 5-day returns (%).
    pub std_dev_pct: Decimal,
    /// Share of positive 5-day moves (%).
    pub positive_pct: Decimal,
    /// 99th percentile 5-day move (%).
    pub p99_pct: Decimal,
    /// 95th percentile 5-day move (%).
    pub p95_pct: Decimal,
    /// Median 5-day move (%).
    pub p50_pct: Decimal,
    /// 5th percentile 5-day move (%).
    pub p5_pct: Decimal,
    /// 1st percentile 5-day move (%).
    pub p1_pct: Decimal,
    /// Average forward 5-day return after `5d >= +tail_threshold_pct` (%).
    pub after_surge_next_5d_avg_pct: Decimal,
    /// Average forward 5-day return after `5d <= -tail_threshold_pct` (%).
    pub after_drop_next_5d_avg_pct: Decimal,
    /// Time series points for chart rendering.
    pub series: Vec<RollingFiveDayPoint>,
}

/// One normalized progress decile (`0%, 10%, ..., 100%`) along a period cycle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleProgressDecile {
    /// Normalized cycle completion percentage (`0, 10, 20, ..., 100`).
    pub progress_pct: u16,
    /// Number of periods contributing to this decile.
    pub sample_count: usize,
    /// Mean cumulative drift from period entry (%).
    pub mean_pct: Decimal,
    /// 10th percentile drift (%).
    pub p10_pct: Decimal,
    /// 25th percentile drift (%).
    pub p25_pct: Decimal,
    /// Median cumulative drift (%).
    pub p50_pct: Decimal,
    /// 75th percentile drift (%).
    pub p75_pct: Decimal,
    /// 90th percentile drift (%).
    pub p90_pct: Decimal,
}

/// Tab 6: Intra-cycle progress (0% → 100%) normalized drift cone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CycleProgressAnalysis {
    /// Number of multi-bar cycles contributing to the cone.
    pub cycle_count: usize,
    /// 11 normalized progress deciles (`0%` through `100%`).
    pub deciles: Vec<CycleProgressDecile>,
}

/// Configuration for the 15-minute Opening Range Breakout + Flip backtester.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrbConfig {
    /// Opening range width in minutes (default `15`).
    pub opening_range_minutes: u16,
    /// Maximum number of reversals/flips permitted per session (default `2`).
    pub max_flips_per_day: usize,
    /// Fixed percentage stop-loss if tighter than the opposite range boundary (`0` disables).
    pub stop_loss_pct: Decimal,
    /// Fixed percentage take-profit target (`0` holds to EOD / flip).
    pub target_pct: Decimal,
    /// Round-trip bid-ask spread slippage deducted per trade (bps).
    pub spread_slippage_bps: Decimal,
    /// Whether to deduct Indian F&O/cash statutory turnover fees per trade.
    pub include_statutory_costs: bool,
}

impl Default for OrbConfig {
    fn default() -> Self {
        Self {
            opening_range_minutes: 15,
            max_flips_per_day: 2,
            stop_loss_pct: Decimal::ZERO,
            target_pct: Decimal::ZERO,
            spread_slippage_bps: Decimal::ZERO,
            include_statutory_costs: false,
        }
    }
}

/// One simulated intraday ORB trade leg.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrbTradeRow {
    /// Session date.
    pub date: NaiveDate,
    /// `"long"` or `"short"`.
    pub direction: String,
    /// Flip sequence inside session (`0` = initial breakout, `1` = first flip, etc.).
    pub flip_index: usize,
    /// Opening range high for the session.
    pub or_high: Decimal,
    /// Opening range low for the session.
    pub or_low: Decimal,
    /// Entry price.
    pub entry_price: Decimal,
    /// Exit price.
    pub exit_price: Decimal,
    /// Exit reason (`"target"`, `"stop_flip"`, `"stop_max_flips"`, `"eod"`).
    pub exit_reason: String,
    /// Gross trade return % before spread & fees.
    pub gross_return_pct: Decimal,
    /// Deducted spread + statutory cost friction (bps).
    pub friction_bps: Decimal,
    /// Net trade return % after spread & fees.
    pub net_return_pct: Decimal,
}

/// Equity curve sample point for ORB backtest visualization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrbEquityPoint {
    /// Session date.
    pub date: NaiveDate,
    /// Cumulative gross return (%).
    pub cumulative_gross_pct: Decimal,
    /// Cumulative net return (%).
    pub cumulative_net_pct: Decimal,
}

/// Tab 10: Complete 15m Opening Range Breakout + Flip simulation result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OrbBacktestResult {
    /// Whether `5m` intraday bars were available to run the simulation.
    pub available: bool,
    /// Total simulated trades across all sessions.
    pub total_trades: usize,
    /// Count of winning trades (`net_return_pct > 0`).
    pub winning_trades: usize,
    /// Win rate (%).
    pub win_rate_pct: Decimal,
    /// Cumulative sum of gross trade returns (%).
    pub gross_return_pct: Decimal,
    /// Total spread + statutory friction deducted (% points).
    pub total_friction_pct: Decimal,
    /// Cumulative sum of net trade returns (%).
    pub net_return_pct: Decimal,
    /// Empirical net profit factor.
    pub profit_factor: Decimal,
    /// Maximum peak-to-trough net return drawdown (%).
    pub max_drawdown_pct: Decimal,
    /// Mean net return per trade (%).
    pub avg_trade_net_pct: Decimal,
    /// Cumulative gross vs net equity curve by session date.
    pub equity_curve: Vec<OrbEquityPoint>,
    /// Most recent simulated trade legs (up to 100).
    pub trades: Vec<OrbTradeRow>,
}

fn format_slot_label(minute_of_day: u16) -> String {
    let h = (minute_of_day / 60) % 24;
    let m = minute_of_day % 60;
    format!("{h:02}:{m:02}")
}

fn bucket_minute_15m(minute_of_day: u16) -> u16 {
    (minute_of_day / 15) * 15
}

fn bucket_minute_hourly(minute_of_day: u16) -> u16 {
    (minute_of_day / 60) * 60
}

#[derive(Default)]
struct SlotAccumulator {
    count: usize,
    range_sum: Decimal,
    max_range: Decimal,
    abs_move_sum: Decimal,
    vol_sum: Decimal,
    hod_count: usize,
    lod_count: usize,
}

/// Computes 15-minute & hourly intraday volatility + HOD/LOD clock distributions.
pub fn analyze_intraday_movement(bars_5m: &[IntradayBar]) -> IntradayMovementAnalysis {
    if bars_5m.is_empty() {
        return IntradayMovementAnalysis::default();
    }

    let mut sessions: BTreeMap<NaiveDate, Vec<&IntradayBar>> = BTreeMap::new();
    for b in bars_5m {
        sessions.entry(b.date).or_default().push(b);
    }

    let hundred = Decimal::from(100);
    let mut map_15m: BTreeMap<u16, SlotAccumulator> = BTreeMap::new();
    let mut map_hourly: BTreeMap<u16, SlotAccumulator> = BTreeMap::new();
    let mut session_range_sum = Decimal::ZERO;
    let mut valid_sessions = 0usize;

    for day_bars in sessions.values() {
        if day_bars.is_empty() {
            continue;
        }
        let mut s_high = day_bars[0].high;
        let mut s_low = day_bars[0].low;
        let s_open = day_bars[0].open;
        let mut hod_min = day_bars[0].minute_of_day;
        let mut lod_min = day_bars[0].minute_of_day;

        for bar in day_bars {
            if bar.high > s_high {
                s_high = bar.high;
                hod_min = bar.minute_of_day;
            }
            if bar.low < s_low {
                s_low = bar.low;
                lod_min = bar.minute_of_day;
            }
            let range_pct = if bar.open > Decimal::ZERO {
                (((bar.high - bar.low) / bar.open) * hundred).round_dp(4)
            } else {
                Decimal::ZERO
            };
            let abs_move = if bar.open > Decimal::ZERO {
                (((bar.close - bar.open).abs() / bar.open) * hundred).round_dp(4)
            } else {
                Decimal::ZERO
            };

            let acc15 = map_15m
                .entry(bucket_minute_15m(bar.minute_of_day))
                .or_default();
            acc15.count += 1;
            acc15.range_sum += range_pct;
            if range_pct > acc15.max_range {
                acc15.max_range = range_pct;
            }
            acc15.abs_move_sum += abs_move;
            acc15.vol_sum += bar.volume;

            let acc_hr = map_hourly
                .entry(bucket_minute_hourly(bar.minute_of_day))
                .or_default();
            acc_hr.count += 1;
            acc_hr.range_sum += range_pct;
            if range_pct > acc_hr.max_range {
                acc_hr.max_range = range_pct;
            }
            acc_hr.abs_move_sum += abs_move;
            acc_hr.vol_sum += bar.volume;
        }

        if s_open > Decimal::ZERO {
            session_range_sum += (((s_high - s_low) / s_open) * hundred).round_dp(4);
            valid_sessions += 1;
        }
        if let Some(a) = map_15m.get_mut(&bucket_minute_15m(hod_min)) {
            a.hod_count += 1;
        }
        if let Some(a) = map_15m.get_mut(&bucket_minute_15m(lod_min)) {
            a.lod_count += 1;
        }
        if let Some(a) = map_hourly.get_mut(&bucket_minute_hourly(hod_min)) {
            a.hod_count += 1;
        }
        if let Some(a) = map_hourly.get_mut(&bucket_minute_hourly(lod_min)) {
            a.lod_count += 1;
        }
    }

    let materialize = |m: BTreeMap<u16, SlotAccumulator>| -> Vec<IntradaySlotStat> {
        m.into_iter()
            .map(|(min, acc)| {
                let d_cnt = Decimal::from(acc.count.max(1) as i64);
                IntradaySlotStat {
                    slot_label: format_slot_label(min),
                    minute_of_day: min,
                    bar_count: acc.count,
                    avg_range_pct: (acc.range_sum / d_cnt).round_dp(3),
                    max_range_pct: acc.max_range.round_dp(3),
                    avg_abs_move_pct: (acc.abs_move_sum / d_cnt).round_dp(3),
                    avg_volume: (acc.vol_sum / d_cnt).round_dp(0),
                    hod_count: acc.hod_count,
                    lod_count: acc.lod_count,
                }
            })
            .collect()
    };

    let slots_15m = materialize(map_15m);
    let slots_hourly = materialize(map_hourly);

    let peak_volatility_slot = slots_15m
        .iter()
        .max_by(|a, b| a.avg_range_pct.cmp(&b.avg_range_pct))
        .map(|s| s.slot_label.clone())
        .unwrap_or_default();
    let most_common_hod_slot = slots_15m
        .iter()
        .max_by_key(|s| s.hod_count)
        .map(|s| s.slot_label.clone())
        .unwrap_or_default();
    let most_common_lod_slot = slots_15m
        .iter()
        .max_by_key(|s| s.lod_count)
        .map(|s| s.slot_label.clone())
        .unwrap_or_default();

    let avg_session_range_pct = if valid_sessions > 0 {
        (session_range_sum / Decimal::from(valid_sessions as i64)).round_dp(2)
    } else {
        Decimal::ZERO
    };

    IntradayMovementAnalysis {
        available: true,
        total_sessions: sessions.len(),
        avg_session_range_pct,
        peak_volatility_slot,
        most_common_hod_slot,
        most_common_lod_slot,
        slots_15m,
        slots_hourly,
    }
}

/// Computes overnight gap frequency, gap-fill probability, and intraday continuation vs fade stats.
pub fn analyze_gaps(daily_bars: &[OhlcvBar], _bars_5m: &[IntradayBar]) -> GapAnalysis {
    if daily_bars.len() < 2 {
        return GapAnalysis::default();
    }

    let hundred = Decimal::from(100);
    let mut events = Vec::with_capacity(daily_bars.len() - 1);

    let mut up_count = 0usize;
    let mut up_fill = 0usize;
    let mut up_gap_sum = Decimal::ZERO;
    let mut up_follow_sum = Decimal::ZERO;

    let mut down_count = 0usize;
    let mut down_fill = 0usize;
    let mut down_gap_sum = Decimal::ZERO;
    let mut down_follow_sum = Decimal::ZERO;

    for pair in daily_bars.windows(2) {
        let prev = &pair[0];
        let curr = &pair[1];
        if prev.close <= Decimal::ZERO || curr.open <= Decimal::ZERO {
            continue;
        }
        let gap_pct = (((curr.open - prev.close) / prev.close) * hundred).round_dp(4);
        let intraday_pct = (((curr.close - curr.open) / curr.open) * hundred).round_dp(4);
        let filled = if gap_pct > Decimal::ZERO {
            curr.low <= prev.close
        } else if gap_pct < Decimal::ZERO {
            curr.high >= prev.close
        } else {
            true
        };

        if gap_pct > Decimal::ZERO {
            up_count += 1;
            up_gap_sum += gap_pct;
            up_follow_sum += intraday_pct;
            if filled {
                up_fill += 1;
            }
        } else if gap_pct < Decimal::ZERO {
            down_count += 1;
            down_gap_sum += gap_pct;
            down_follow_sum += intraday_pct;
            if filled {
                down_fill += 1;
            }
        }

        events.push(GapEventRow {
            date: curr.date,
            prev_close: prev.close,
            open: curr.open,
            close: curr.close,
            gap_pct,
            intraday_return_pct: intraday_pct,
            filled,
        });
    }

    let bucket_defs: [(&str, Option<Decimal>, Option<Decimal>); 8] = [
        ("< -2.0%", None, Some(Decimal::new(-2, 0))),
        (
            "-2.0% to -1.0%",
            Some(Decimal::new(-2, 0)),
            Some(Decimal::new(-1, 0)),
        ),
        (
            "-1.0% to -0.5%",
            Some(Decimal::new(-1, 0)),
            Some(Decimal::new(-5, 1)),
        ),
        (
            "-0.5% to 0.0%",
            Some(Decimal::new(-5, 1)),
            Some(Decimal::ZERO),
        ),
        (
            "0.0% to +0.5%",
            Some(Decimal::ZERO),
            Some(Decimal::new(5, 1)),
        ),
        (
            "+0.5% to +1.0%",
            Some(Decimal::new(5, 1)),
            Some(Decimal::ONE),
        ),
        (
            "+1.0% to +2.0%",
            Some(Decimal::ONE),
            Some(Decimal::new(2, 0)),
        ),
        ("> +2.0%", Some(Decimal::new(2, 0)), None),
    ];

    let total = events.len().max(1);
    let total_dec = Decimal::from(total as i64);
    let mut buckets = Vec::with_capacity(bucket_defs.len());

    for (label, lo, hi) in bucket_defs {
        let mut count = 0usize;
        let mut fills = 0usize;
        let mut follow_sum = Decimal::ZERO;
        for ev in &events {
            let matches = match (lo, hi) {
                (None, Some(h)) => ev.gap_pct < h,
                (Some(l), Some(h)) => ev.gap_pct >= l && ev.gap_pct < h,
                (Some(l), None) => ev.gap_pct >= l,
                (None, None) => true,
            };
            if matches {
                count += 1;
                if ev.filled {
                    fills += 1;
                }
                follow_sum += ev.intraday_return_pct;
            }
        }
        let share_pct = ((Decimal::from(count as i64) / total_dec) * hundred).round_dp(2);
        let fill_rate_pct = if count > 0 {
            ((Decimal::from(fills as i64) / Decimal::from(count as i64)) * hundred).round_dp(1)
        } else {
            Decimal::ZERO
        };
        let avg_follow = if count > 0 {
            (follow_sum / Decimal::from(count as i64)).round_dp(3)
        } else {
            Decimal::ZERO
        };
        buckets.push(GapBucketRow {
            label: label.to_string(),
            count,
            share_pct,
            fill_rate_pct,
            avg_intraday_follow_pct: avg_follow,
        });
    }

    let recent_events: Vec<GapEventRow> = events.iter().rev().take(40).cloned().collect();

    let safe_div = |num: Decimal, den: usize| -> Decimal {
        if den == 0 {
            Decimal::ZERO
        } else {
            (num / Decimal::from(den as i64)).round_dp(2)
        }
    };
    let safe_rate = |num: usize, den: usize| -> Decimal {
        if den == 0 {
            Decimal::ZERO
        } else {
            ((Decimal::from(num as i64) / Decimal::from(den as i64)) * hundred).round_dp(1)
        }
    };

    GapAnalysis {
        total_sessions: events.len(),
        gap_up_count: up_count,
        gap_down_count: down_count,
        avg_gap_up_pct: safe_div(up_gap_sum, up_count),
        avg_gap_down_pct: safe_div(down_gap_sum, down_count),
        gap_up_fill_rate_pct: safe_rate(up_fill, up_count),
        gap_down_fill_rate_pct: safe_rate(down_fill, down_count),
        gap_up_follow_pct: safe_div(up_follow_sum, up_count),
        gap_down_follow_pct: safe_div(down_follow_sum, down_count),
        buckets,
        recent_events,
    }
}

/// Computes rolling 5-trading-day move distribution and exhaustion response.
pub fn analyze_rolling_5day(
    daily_bars: &[OhlcvBar],
    tail_threshold_pct: Decimal,
) -> RollingFiveDayAnalysis {
    if daily_bars.len() < 6 {
        return RollingFiveDayAnalysis::default();
    }

    let hundred = Decimal::from(100);
    let mut series = Vec::with_capacity(daily_bars.len() - 5);
    for idx in 5..daily_bars.len() {
        let base = daily_bars[idx - 5].close;
        let curr = &daily_bars[idx];
        if base <= Decimal::ZERO {
            continue;
        }
        let ret = (((curr.close - base) / base) * hundred).round_dp(4);
        series.push(RollingFiveDayPoint {
            date: curr.date,
            close: curr.close,
            return_5d_pct: ret,
        });
    }

    if series.is_empty() {
        return RollingFiveDayAnalysis::default();
    }

    let returns: Vec<Decimal> = series.iter().map(|s| s.return_5d_pct).collect();
    let (avg, std_dev) = mean_and_sample_std(&returns);
    let pos_count = returns.iter().filter(|r| **r > Decimal::ZERO).count();
    let pos_pct = ((Decimal::from(pos_count as i64) / Decimal::from(returns.len() as i64))
        * hundred)
        .round_dp(1);

    let mut sorted = returns.clone();
    sorted.sort();

    let mut surge_next = Vec::new();
    let mut drop_next = Vec::new();
    for i in 0..series.len() {
        if i + 5 >= series.len() {
            continue;
        }
        let r = series[i].return_5d_pct;
        let next_r = series[i + 5].return_5d_pct;
        if r >= tail_threshold_pct {
            surge_next.push(next_r);
        } else if r <= -tail_threshold_pct {
            drop_next.push(next_r);
        }
    }

    let (surge_avg, _) = mean_and_sample_std(&surge_next);
    let (drop_avg, _) = mean_and_sample_std(&drop_next);

    RollingFiveDayAnalysis {
        sample_count: series.len(),
        avg_return_pct: avg.round_dp(2),
        std_dev_pct: std_dev.round_dp(2),
        positive_pct: pos_pct,
        p99_pct: empirical_quantile(&sorted, Decimal::from(99)).round_dp(2),
        p95_pct: empirical_quantile(&sorted, Decimal::from(95)).round_dp(2),
        p50_pct: empirical_quantile(&sorted, Decimal::from(50)).round_dp(2),
        p5_pct: empirical_quantile(&sorted, Decimal::from(5)).round_dp(2),
        p1_pct: empirical_quantile(&sorted, Decimal::from(1)).round_dp(2),
        after_surge_next_5d_avg_pct: surge_avg.round_dp(2),
        after_drop_next_5d_avg_pct: drop_avg.round_dp(2),
        series,
    }
}

/// Computes normalized `0% → 100%` intra-cycle drift cone across sliced periods.
pub fn analyze_cycle_progress(
    daily_bars: &[OhlcvBar],
    periods: &[SlicedPeriod],
) -> CycleProgressAnalysis {
    if daily_bars.is_empty() || periods.is_empty() {
        return CycleProgressAnalysis::default();
    }

    let date_idx: HashMap<NaiveDate, usize> = daily_bars
        .iter()
        .enumerate()
        .map(|(i, b)| (b.date, i))
        .collect();
    let hundred = Decimal::from(100);

    // 11 decile buckets: 0%, 10%, ..., 100%
    let mut decile_samples: [Vec<Decimal>; 11] = Default::default();
    let mut cycle_count = 0usize;

    for p in periods {
        if p.entry_price <= Decimal::ZERO {
            continue;
        }
        let (Some(&start_i), Some(&end_i)) =
            (date_idx.get(&p.start_date), date_idx.get(&p.end_date))
        else {
            continue;
        };
        if end_i <= start_i {
            continue;
        }
        let span = end_i - start_i;
        cycle_count += 1;

        for (d_idx, bucket) in decile_samples.iter_mut().enumerate() {
            // Normalized position along [0..span]
            let target_pos =
                Decimal::from(d_idx as i64) * Decimal::from(span as i64) / Decimal::from(10);
            let lo = target_pos.floor().to_string().parse::<usize>().unwrap_or(0);
            let lo = lo.min(span);
            let hi = (lo + 1).min(span);
            let frac = target_pos - Decimal::from(lo as i64);

            let c_lo = daily_bars[start_i + lo].close;
            let c_hi = daily_bars[start_i + hi].close;
            let interp_price = c_lo + (c_hi - c_lo) * frac;
            let cum_ret = (((interp_price - p.entry_price) / p.entry_price) * hundred).round_dp(4);
            bucket.push(cum_ret);
        }
    }

    if cycle_count == 0 {
        return CycleProgressAnalysis::default();
    }

    let mut deciles = Vec::with_capacity(11);
    for (idx, samples) in decile_samples.iter_mut().enumerate() {
        samples.sort();
        let (mean, _) = mean_and_sample_std(samples);
        deciles.push(CycleProgressDecile {
            progress_pct: (idx as u16) * 10,
            sample_count: samples.len(),
            mean_pct: mean.round_dp(2),
            p10_pct: empirical_quantile(samples, Decimal::from(10)).round_dp(2),
            p25_pct: empirical_quantile(samples, Decimal::from(25)).round_dp(2),
            p50_pct: empirical_quantile(samples, Decimal::from(50)).round_dp(2),
            p75_pct: empirical_quantile(samples, Decimal::from(75)).round_dp(2),
            p90_pct: empirical_quantile(samples, Decimal::from(90)).round_dp(2),
        });
    }

    CycleProgressAnalysis {
        cycle_count,
        deciles,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PositionDir {
    Long,
    Short,
}

impl PositionDir {
    fn label(self) -> &'static str {
        match self {
            PositionDir::Long => "long",
            PositionDir::Short => "short",
        }
    }

    fn flip(self) -> Self {
        match self {
            PositionDir::Long => PositionDir::Short,
            PositionDir::Short => PositionDir::Long,
        }
    }
}

/// Runs the 15-minute Opening Range Breakout + flip-on-SL simulation with F&O spread & cost deductions.
pub fn run_orb15_flip_backtest(
    bars_5m: &[IntradayBar],
    asset_class: StudyAssetClass,
    exchange_or_symbol: &str,
    cfg: &OrbConfig,
) -> OrbBacktestResult {
    if bars_5m.is_empty() {
        return OrbBacktestResult::default();
    }

    let mut sessions: BTreeMap<NaiveDate, Vec<&IntradayBar>> = BTreeMap::new();
    for b in bars_5m {
        sessions.entry(b.date).or_default().push(b);
    }

    let hundred = Decimal::from(100);
    let or_mins = cfg.opening_range_minutes.max(5);
    let stat_bps = if cfg.include_statutory_costs {
        estimate_statutory_roundtrip_bps(exchange_or_symbol, asset_class)
    } else {
        Decimal::ZERO
    };
    let per_trade_friction_bps =
        (cfg.spread_slippage_bps.max(Decimal::ZERO) + stat_bps).round_dp(2);
    let per_trade_friction_pct = (per_trade_friction_bps / hundred).round_dp(4);

    let mut all_trades: Vec<OrbTradeRow> = Vec::new();
    let mut equity_curve: Vec<OrbEquityPoint> = Vec::new();
    let mut cum_gross = Decimal::ZERO;
    let mut cum_net = Decimal::ZERO;
    let mut total_friction = Decimal::ZERO;

    for (date, mut day_bars) in sessions {
        day_bars.sort_by_key(|b| b.minute_of_day);
        if day_bars.len() < 3 {
            continue;
        }
        let start_min = day_bars[0].minute_of_day;
        let or_end_min = start_min.saturating_add(or_mins);

        let or_bars: Vec<&&IntradayBar> = day_bars
            .iter()
            .filter(|b| b.minute_of_day < or_end_min)
            .collect();
        let trade_bars: Vec<&&IntradayBar> = day_bars
            .iter()
            .filter(|b| b.minute_of_day >= or_end_min)
            .collect();
        if or_bars.is_empty() || trade_bars.is_empty() {
            continue;
        }

        let mut or_high = or_bars[0].high;
        let mut or_low = or_bars[0].low;
        for b in &or_bars {
            if b.high > or_high {
                or_high = b.high;
            }
            if b.low < or_low {
                or_low = b.low;
            }
        }
        if or_high <= Decimal::ZERO || or_low <= Decimal::ZERO || or_high <= or_low {
            continue;
        }

        struct ActivePos {
            dir: PositionDir,
            entry: Decimal,
            stop: Decimal,
            target: Option<Decimal>,
            flip_idx: usize,
        }

        let make_pos = |dir: PositionDir, entry: Decimal, flip_idx: usize| -> ActivePos {
            let sl_dist_pct = if cfg.stop_loss_pct > Decimal::ZERO {
                Some(entry * (cfg.stop_loss_pct / hundred))
            } else {
                None
            };
            let stop = match dir {
                PositionDir::Long => {
                    let sl = sl_dist_pct
                        .map(|d| (entry - d).max(or_low))
                        .unwrap_or(or_low);
                    sl.min(entry)
                }
                PositionDir::Short => {
                    let sl = sl_dist_pct
                        .map(|d| (entry + d).min(or_high))
                        .unwrap_or(or_high);
                    sl.max(entry)
                }
            };
            let target = if cfg.target_pct > Decimal::ZERO {
                let tgt_delta = entry * (cfg.target_pct / hundred);
                Some(match dir {
                    PositionDir::Long => entry + tgt_delta,
                    PositionDir::Short => (entry - tgt_delta).max(Decimal::ZERO),
                })
            } else {
                None
            };
            ActivePos {
                dir,
                entry,
                stop,
                target,
                flip_idx,
            }
        };

        let mut pos: Option<ActivePos> = None;
        let mut flips_used = 0usize;
        let mut halted_for_day = false;
        let mut day_gross = Decimal::ZERO;
        let mut day_net = Decimal::ZERO;

        let last_idx = trade_bars.len() - 1;
        for (i, bar) in trade_bars.iter().enumerate() {
            if halted_for_day {
                break;
            }

            if let Some(active) = pos.take() {
                let hit_stop = match active.dir {
                    PositionDir::Long => bar.low <= active.stop,
                    PositionDir::Short => bar.high >= active.stop,
                };
                let hit_target = active.target.is_some_and(|tgt| match active.dir {
                    PositionDir::Long => bar.high >= tgt,
                    PositionDir::Short => bar.low <= tgt,
                });

                if hit_stop {
                    let exit = active.stop;
                    let gross = match active.dir {
                        PositionDir::Long => ((exit - active.entry) / active.entry) * hundred,
                        PositionDir::Short => ((active.entry - exit) / active.entry) * hundred,
                    }
                    .round_dp(4);
                    let net = (gross - per_trade_friction_pct).round_dp(4);
                    day_gross += gross;
                    day_net += net;
                    total_friction += per_trade_friction_pct;

                    let can_flip = flips_used < cfg.max_flips_per_day && i < last_idx;
                    all_trades.push(OrbTradeRow {
                        date,
                        direction: active.dir.label().to_string(),
                        flip_index: active.flip_idx,
                        or_high,
                        or_low,
                        entry_price: active.entry,
                        exit_price: exit,
                        exit_reason: if can_flip {
                            "stop_flip".to_string()
                        } else {
                            "stop_max_flips".to_string()
                        },
                        gross_return_pct: gross,
                        friction_bps: per_trade_friction_bps,
                        net_return_pct: net,
                    });

                    if can_flip {
                        flips_used += 1;
                        pos = Some(make_pos(active.dir.flip(), exit, flips_used));
                    } else {
                        halted_for_day = true;
                    }
                    continue;
                }

                if hit_target {
                    let exit = active.target.unwrap();
                    let gross = match active.dir {
                        PositionDir::Long => ((exit - active.entry) / active.entry) * hundred,
                        PositionDir::Short => ((active.entry - exit) / active.entry) * hundred,
                    }
                    .round_dp(4);
                    let net = (gross - per_trade_friction_pct).round_dp(4);
                    day_gross += gross;
                    day_net += net;
                    total_friction += per_trade_friction_pct;

                    all_trades.push(OrbTradeRow {
                        date,
                        direction: active.dir.label().to_string(),
                        flip_index: active.flip_idx,
                        or_high,
                        or_low,
                        entry_price: active.entry,
                        exit_price: exit,
                        exit_reason: "target".to_string(),
                        gross_return_pct: gross,
                        friction_bps: per_trade_friction_bps,
                        net_return_pct: net,
                    });
                    halted_for_day = true;
                    continue;
                }

                if i == last_idx {
                    let exit = bar.close;
                    let gross = match active.dir {
                        PositionDir::Long => ((exit - active.entry) / active.entry) * hundred,
                        PositionDir::Short => ((active.entry - exit) / active.entry) * hundred,
                    }
                    .round_dp(4);
                    let net = (gross - per_trade_friction_pct).round_dp(4);
                    day_gross += gross;
                    day_net += net;
                    total_friction += per_trade_friction_pct;

                    all_trades.push(OrbTradeRow {
                        date,
                        direction: active.dir.label().to_string(),
                        flip_index: active.flip_idx,
                        or_high,
                        or_low,
                        entry_price: active.entry,
                        exit_price: exit,
                        exit_reason: "eod".to_string(),
                        gross_return_pct: gross,
                        friction_bps: per_trade_friction_bps,
                        net_return_pct: net,
                    });
                    continue;
                }

                pos = Some(active);
            } else if i < last_idx {
                if bar.close > or_high {
                    pos = Some(make_pos(PositionDir::Long, bar.close, 0));
                } else if bar.close < or_low {
                    pos = Some(make_pos(PositionDir::Short, bar.close, 0));
                }
            }
        }

        cum_gross += day_gross;
        cum_net += day_net;
        equity_curve.push(OrbEquityPoint {
            date,
            cumulative_gross_pct: cum_gross.round_dp(2),
            cumulative_net_pct: cum_net.round_dp(2),
        });
    }

    let total_trades = all_trades.len();
    let winning_trades = all_trades
        .iter()
        .filter(|t| t.net_return_pct > Decimal::ZERO)
        .count();
    let win_rate_pct = if total_trades > 0 {
        ((Decimal::from(winning_trades as i64) / Decimal::from(total_trades as i64)) * hundred)
            .round_dp(1)
    } else {
        Decimal::ZERO
    };

    let mut pos_sum = Decimal::ZERO;
    let mut neg_abs_sum = Decimal::ZERO;
    for t in &all_trades {
        if t.net_return_pct > Decimal::ZERO {
            pos_sum += t.net_return_pct;
        } else if t.net_return_pct < Decimal::ZERO {
            neg_abs_sum += t.net_return_pct.abs();
        }
    }
    let profit_factor = if neg_abs_sum.is_zero() {
        if pos_sum > Decimal::ZERO {
            Decimal::from(99)
        } else {
            Decimal::ZERO
        }
    } else {
        (pos_sum / neg_abs_sum).round_dp(2)
    };

    let mut peak = Decimal::ZERO;
    let mut max_dd = Decimal::ZERO;
    for pt in &equity_curve {
        if pt.cumulative_net_pct > peak {
            peak = pt.cumulative_net_pct;
        }
        let dd = peak - pt.cumulative_net_pct;
        if dd > max_dd {
            max_dd = dd;
        }
    }

    let avg_trade_net_pct = if total_trades > 0 {
        (cum_net / Decimal::from(total_trades as i64)).round_dp(3)
    } else {
        Decimal::ZERO
    };

    // Unused helper reference guard for deterministic builds
    let _ = decimal_sqrt(Decimal::ONE);

    let recent_trades: Vec<OrbTradeRow> = all_trades.into_iter().rev().take(100).collect();

    OrbBacktestResult {
        available: true,
        total_trades,
        winning_trades,
        win_rate_pct,
        gross_return_pct: cum_gross.round_dp(2),
        total_friction_pct: total_friction.round_dp(2),
        net_return_pct: cum_net.round_dp(2),
        profit_factor,
        max_drawdown_pct: max_dd.round_dp(2),
        avg_trade_net_pct,
        equity_curve,
        trades: recent_trades,
    }
}
