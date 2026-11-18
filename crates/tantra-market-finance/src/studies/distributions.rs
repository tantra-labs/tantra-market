//! Empirical percentiles, coarse + 10-bps distributions, probability-based levels,
//! Green-vs-Red splits, conditional forward moves, and survivability outlier detection.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::returns::mean_and_sample_std;
use super::schedule::SlicedPeriod;

/// Seven empirical quantile landmarks (99th .. 1st).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReturnPercentiles {
    /// 99th percentile (Extreme Positive).
    pub p99: Decimal,
    /// 95th percentile (Very Positive).
    pub p95: Decimal,
    /// 75th percentile (Positive).
    pub p75: Decimal,
    /// 50th percentile (Median).
    pub p50: Decimal,
    /// 25th percentile (Negative).
    pub p25: Decimal,
    /// 5th percentile (Very Negative).
    pub p5: Decimal,
    /// 1st percentile (Extreme Negative).
    pub p1: Decimal,
}

/// One row of the 16-bucket or 10-bps Return Distribution table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnBucketRow {
    /// Human range label (e.g. `"> +5%"`, `"1.0% to 2.0%"`).
    pub label: String,
    /// Inclusive/exclusive lower bound (%).
    pub min_pct: Option<Decimal>,
    /// Upper bound (%).
    pub max_pct: Option<Decimal>,
    /// Number of periods landing in this bucket.
    pub count: usize,
    /// Percentage of total periods (`count / total * 100`).
    pub pct_of_total: Decimal,
    /// Cumulative quantile range label (e.g. `"Top 0.2%"`, `"14.2% - 33.5%"`).
    pub percentile_range: String,
    /// Cumulative exceedance/directional probability label (`"46.2% - 66.0%"`).
    pub probability_label: String,
    /// Projected lower spot price anchored to reference price.
    pub price_range_low: Option<Decimal>,
    /// Projected upper spot price anchored to reference price.
    pub price_range_high: Option<Decimal>,
}

/// One exceedance row in the `Probability-Based Levels (Easy Reference)` table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbabilityLevelRow {
    /// Empirical probability (%) spot closes ABOVE `implied_price`.
    pub prob_above_pct: usize,
    /// Return threshold (%).
    pub return_pct: Decimal,
    /// Implied spot price anchored to reference price (`ref_price * (1 + return_pct/100)`).
    pub implied_price: Decimal,
    /// Change from reference (%).
    pub change_pct: Decimal,
    /// Directional scenario classification tag.
    pub scenario: String,
}

/// Descriptive statistics for either positive (`Green`) or negative (`Red`) periods.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PolarityStats {
    /// Period count on this side.
    pub count: usize,
    /// Percentage share across all periods.
    pub share_pct: Decimal,
    /// Mean return (%) on this side.
    pub mean_pct: Decimal,
    /// Median return (%) on this side.
    pub median_pct: Decimal,
    /// Sample standard deviation (%) on this side.
    pub std_dev_pct: Decimal,
    /// Largest magnitude return (%) on this side.
    pub extreme_pct: Decimal,
}

/// One magnitude bucket in the Green vs Red PDF comparison bar chart & tables.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MagnitudePdfBucket {
    /// Magnitude label (`"0% - 1%"`, `"1% - 2%"`, ..., `"> 5%"`).
    pub label: String,
    /// Count of Green periods in magnitude band.
    pub green_count: usize,
    /// Share (%) among Green periods.
    pub green_pct: Decimal,
    /// Count of Red periods in magnitude band.
    pub red_count: usize,
    /// Share (%) among Red periods.
    pub red_pct: Decimal,
}

/// Green vs Red split comparison bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GreenVsRedComparison {
    /// Summary stats for Green periods (`r > 0`).
    pub green: PolarityStats,
    /// Summary stats for Red periods (`r < 0`).
    pub red: PolarityStats,
    /// Magnitude bands comparing Green vs Red empirical densities.
    pub magnitude_buckets: Vec<MagnitudePdfBucket>,
}

/// Single observation of a next-period return conditioned on a large trigger period.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardConditionEvent {
    /// Date the large trigger period ended.
    pub trigger_end_date: NaiveDate,
    /// Return of the trigger period (%).
    pub trigger_return_pct: Decimal,
    /// End date of the immediately following period.
    pub next_end_date: NaiveDate,
    /// Return of the immediately following period (%).
    pub next_return_pct: Decimal,
}

/// Aggregate forward expectancy following large positive or large negative moves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ForwardConditionalGroup {
    /// Trigger threshold applied (e.g. `2.00` for `|r| >= 2%`).
    pub threshold_pct: Decimal,
    /// Mean forward return (%) after `r >= +threshold`.
    pub after_large_pos_mean_pct: Decimal,
    /// Median forward return (%) after `r >= +threshold`.
    pub after_large_pos_median_pct: Decimal,
    /// Win-rate (%) for next period being positive after `r >= +threshold`.
    pub after_large_pos_win_pct: Decimal,
    /// Events following `r >= +threshold`.
    pub after_large_pos_events: Vec<ForwardConditionEvent>,
    /// Mean forward return (%) after `r <= -threshold`.
    pub after_large_neg_mean_pct: Decimal,
    /// Median forward return (%) after `r <= -threshold`.
    pub after_large_neg_median_pct: Decimal,
    /// Win-rate (%) for next period being positive after `r <= -threshold`.
    pub after_large_neg_win_pct: Decimal,
    /// Events following `r <= -threshold`.
    pub after_large_neg_events: Vec<ForwardConditionEvent>,
}

/// Survivability outlier classification for Tab 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutlierStatus {
    /// Return falls inside symmetric `[lower_bound, upper_bound]`.
    WithinBoundary,
    /// Return strictly exceeds `upper_bound`.
    UpsideOutlier,
    /// Return strictly below `lower_bound`.
    DownsideOutlier,
}

/// Single period point with outlier classification for the Survivability Scatter Plot & table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlierPeriodPoint {
    /// Cycle index.
    pub index: usize,
    /// Period start date.
    pub start_date: NaiveDate,
    /// Period end date.
    pub end_date: NaiveDate,
    /// Period return (%).
    pub return_pct: Decimal,
    /// Classification under the coverage slider.
    pub status: OutlierStatus,
}

/// 4-KPI outlier strip summary + boundary thresholds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutlierKpis {
    /// Configured coverage percentage (`50..=99`).
    pub coverage_pct: Decimal,
    /// Symmetric upper threshold (%).
    pub upper_bound_pct: Decimal,
    /// Symmetric lower threshold (%).
    pub lower_bound_pct: Decimal,
    /// Total periods evaluated.
    pub total_periods: usize,
    /// Total outliers (`upside + downside`).
    pub total_outliers: usize,
    /// Outlier percentage (`total_outliers / total_periods * 100`).
    pub outlier_pct: Decimal,
    /// Upside outliers count.
    pub upside_outliers: usize,
    /// Downside outliers count.
    pub downside_outliers: usize,
}

/// Complete Survivability Outlier analysis bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutlierAnalysis {
    /// Summary KPIs & computed bounds.
    pub kpis: OutlierKpis,
    /// All periods tagged with outlier status (chronological, for scatter plot).
    pub scatter_points: Vec<OutlierPeriodPoint>,
    /// Filtered outlier periods sorted by `|return_pct|` descending (for Outlier table + event lookup).
    pub outlier_table: Vec<OutlierPeriodPoint>,
}

/// Combined empirical distribution + reference-level + forward + outlier output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DistributionBundle {
    /// 7 quantile levels (`P99..P1`).
    pub percentiles: ReturnPercentiles,
    /// Current latest spot price.
    pub current_price: Decimal,
    /// Current period opening reference price.
    pub current_reference_price: Decimal,
    /// Live reference change % (`(current - ref) / ref * 100`).
    pub current_reference_change_pct: Decimal,
    /// 16 coarse color-coded return buckets.
    pub coarse_buckets: Vec<ReturnBucketRow>,
    /// Non-empty 10-bps (`0.1%`) granular return buckets sorted descending.
    pub granular_10bps: Vec<ReturnBucketRow>,
    /// Empirical exceedance price levels (`10%..99%`).
    pub probability_levels: Vec<ProbabilityLevelRow>,
    /// Split Green vs Red statistics & PDF buckets.
    pub green_vs_red: GreenVsRedComparison,
    /// Forward conditional returns after large trigger moves.
    pub forward_conditional: ForwardConditionalGroup,
    /// Survivability outlier scatter & sorted table.
    pub outliers: OutlierAnalysis,
}

/// Computes linear-interpolated quantile `p` (`0..=100`) on a pre-sorted ascending slice.
pub fn empirical_quantile(sorted_asc: &[Decimal], p: Decimal) -> Decimal {
    if sorted_asc.is_empty() {
        return Decimal::ZERO;
    }
    if sorted_asc.len() == 1 {
        return sorted_asc[0].round_dp(2);
    }
    let hundred = Decimal::from(100);
    let clamped = p.max(Decimal::ZERO).min(hundred);
    let n_minus_1 = Decimal::from((sorted_asc.len() - 1) as i64);
    let pos = ((clamped / hundred) * n_minus_1).round_dp(8);

    let lo_idx_dec = pos.floor();
    let lo_idx = lo_idx_dec
        .to_string()
        .parse::<usize>()
        .unwrap_or(0)
        .min(sorted_asc.len() - 1);
    let hi_idx = (lo_idx + 1).min(sorted_asc.len() - 1);
    let frac = pos - Decimal::from(lo_idx as i64);
    let interp = sorted_asc[lo_idx] + frac * (sorted_asc[hi_idx] - sorted_asc[lo_idx]);
    interp.round_dp(2)
}

fn median_of(values: &[Decimal]) -> Decimal {
    if values.is_empty() {
        return Decimal::ZERO;
    }
    let mut s = values.to_vec();
    s.sort();
    empirical_quantile(&s, Decimal::from(50))
}

fn price_from_return(ref_price: Decimal, ret_pct: Decimal) -> Decimal {
    if ref_price.is_zero() {
        Decimal::ZERO
    } else {
        (ref_price * (Decimal::ONE + (ret_pct / Decimal::from(100)))).round_dp(2)
    }
}

fn scenario_tag(ret_pct: Decimal) -> &'static str {
    if ret_pct >= Decimal::new(300, 2) {
        "Strong Upside"
    } else if ret_pct >= Decimal::from(1) {
        "Moderate Upside"
    } else if ret_pct >= Decimal::ZERO {
        "Slight Upside"
    } else if ret_pct > Decimal::from(-1) {
        "Slight Downside"
    } else if ret_pct > Decimal::from(-3) {
        "Moderate Downside"
    } else {
        "Strong Downside"
    }
}

/// Evaluates all distributions, reference levels, forward studies, and survivability outliers.
pub fn analyze_distributions_and_outliers(
    periods: &[SlicedPeriod],
    current_price: Decimal,
    current_reference_price: Decimal,
    coverage_pct: Decimal,
    tail_threshold_pct: Decimal,
) -> DistributionBundle {
    if periods.is_empty() {
        return DistributionBundle::default();
    }

    let hundred = Decimal::from(100);
    let mut sorted_asc: Vec<Decimal> = periods.iter().map(|p| p.return_pct).collect();
    sorted_asc.sort();
    let total = sorted_asc.len();
    let total_dec = Decimal::from(total as i64);

    let percentiles = ReturnPercentiles {
        p99: empirical_quantile(&sorted_asc, Decimal::from(99)),
        p95: empirical_quantile(&sorted_asc, Decimal::from(95)),
        p75: empirical_quantile(&sorted_asc, Decimal::from(75)),
        p50: empirical_quantile(&sorted_asc, Decimal::from(50)),
        p25: empirical_quantile(&sorted_asc, Decimal::from(25)),
        p5: empirical_quantile(&sorted_asc, Decimal::from(5)),
        p1: empirical_quantile(&sorted_asc, Decimal::from(1)),
    };

    let ref_price = if current_reference_price > Decimal::ZERO {
        current_reference_price
    } else {
        periods.last().unwrap().entry_price
    };
    let spot = if current_price > Decimal::ZERO {
        current_price
    } else {
        periods.last().unwrap().exit_price
    };
    let ref_change_pct = if ref_price.is_zero() {
        Decimal::ZERO
    } else {
        (((spot - ref_price) / ref_price) * hundred).round_dp(2)
    };

    // 16 Coarse Buckets (> +5% down to < -5%)
    let coarse_defs: Vec<(&str, Option<Decimal>, Option<Decimal>)> = vec![
        ("> 5%", Some(Decimal::new(50, 1)), None),
        (
            "4.5% to 5.0%",
            Some(Decimal::new(45, 1)),
            Some(Decimal::new(50, 1)),
        ),
        (
            "4.0% to 4.5%",
            Some(Decimal::new(40, 1)),
            Some(Decimal::new(45, 1)),
        ),
        (
            "3.5% to 4.0%",
            Some(Decimal::new(35, 1)),
            Some(Decimal::new(40, 1)),
        ),
        (
            "3.0% to 3.5%",
            Some(Decimal::new(30, 1)),
            Some(Decimal::new(35, 1)),
        ),
        (
            "2.0% to 3.0%",
            Some(Decimal::new(20, 1)),
            Some(Decimal::new(30, 1)),
        ),
        (
            "1.0% to 2.0%",
            Some(Decimal::new(10, 1)),
            Some(Decimal::new(20, 1)),
        ),
        (
            "0.0% to 1.0%",
            Some(Decimal::ZERO),
            Some(Decimal::new(10, 1)),
        ),
        (
            "-1.0% to 0.0%",
            Some(Decimal::new(-10, 1)),
            Some(Decimal::ZERO),
        ),
        (
            "-2.0% to -1.0%",
            Some(Decimal::new(-20, 1)),
            Some(Decimal::new(-10, 1)),
        ),
        (
            "-3.0% to -2.0%",
            Some(Decimal::new(-30, 1)),
            Some(Decimal::new(-20, 1)),
        ),
        (
            "-3.5% to -3.0%",
            Some(Decimal::new(-35, 1)),
            Some(Decimal::new(-30, 1)),
        ),
        (
            "-4.0% to -3.5%",
            Some(Decimal::new(-40, 1)),
            Some(Decimal::new(-35, 1)),
        ),
        (
            "-4.5% to -4.0%",
            Some(Decimal::new(-45, 1)),
            Some(Decimal::new(-40, 1)),
        ),
        (
            "-5.0% to -4.5%",
            Some(Decimal::new(-50, 1)),
            Some(Decimal::new(-45, 1)),
        ),
        ("< -5%", None, Some(Decimal::new(-50, 1))),
    ];

    let mut cum_count = 0usize;
    let mut coarse_buckets = Vec::with_capacity(coarse_defs.len());
    for (idx, (label, min_b, max_b)) in coarse_defs.iter().enumerate() {
        let count = sorted_asc
            .iter()
            .filter(|&&r| match (min_b, max_b) {
                (Some(lo), None) => r > *lo,
                (None, Some(hi)) => r < *hi,
                (Some(lo), Some(hi)) => {
                    if *lo == Decimal::ZERO {
                        r >= *lo && r <= *hi
                    } else {
                        r >= *lo && r < *hi
                    }
                }
                (None, None) => false,
            })
            .count();

        let pct_of_total = ((Decimal::from(count as i64) / total_dec) * hundred).round_dp(1);
        let start_cum = ((Decimal::from(cum_count as i64) / total_dec) * hundred).round_dp(1);
        cum_count += count;
        let end_cum = ((Decimal::from(cum_count as i64) / total_dec) * hundred).round_dp(1);

        let (percentile_range, probability_label) = if count == 0 {
            ("0.0%".to_string(), "0.0%".to_string())
        } else if idx == 0 {
            (
                format!("Top {end_cum}%"),
                format!("{:.1}%", Decimal::from(100) - start_cum),
            )
        } else if idx == coarse_defs.len() - 1 {
            ("Bottom 100.0%".to_string(), "Bottom 100.0%".to_string())
        } else {
            (
                format!("{start_cum}% - {end_cum}%"),
                format!(
                    "{:.1}% - {:.1}%",
                    (hundred - end_cum).max(Decimal::ZERO),
                    (hundred - start_cum).max(Decimal::ZERO)
                ),
            )
        };

        coarse_buckets.push(ReturnBucketRow {
            label: (*label).to_string(),
            min_pct: *min_b,
            max_pct: *max_b,
            count,
            pct_of_total,
            percentile_range,
            probability_label,
            price_range_low: min_b.map(|m| price_from_return(ref_price, m)),
            price_range_high: max_b.map(|m| price_from_return(ref_price, m)),
        });
    }

    // 10-bps (0.1%) Granular Distribution sorted highest-to-lowest
    let mut bps_bins: BTreeMap<i32, usize> = BTreeMap::new();
    for &r in &sorted_asc {
        // Scale by 10 -> integer tenths of a percent (`floor(r * 10)`)
        let tenths = (r * Decimal::from(10))
            .floor()
            .to_string()
            .parse::<i32>()
            .unwrap_or(0);
        *bps_bins.entry(tenths).or_insert(0) += 1;
    }

    let mut granular_cum = 0usize;
    let mut granular_10bps = Vec::with_capacity(bps_bins.len());
    for (&bin_tenths, &count) in bps_bins.iter().rev() {
        let lo = Decimal::from(bin_tenths) / Decimal::from(10);
        let hi = Decimal::from(bin_tenths + 1) / Decimal::from(10);
        let pct_of_total = ((Decimal::from(count as i64) / total_dec) * hundred).round_dp(2);
        let start_cum = ((Decimal::from(granular_cum as i64) / total_dec) * hundred).round_dp(1);
        granular_cum += count;
        let end_cum = ((Decimal::from(granular_cum as i64) / total_dec) * hundred).round_dp(1);
        granular_10bps.push(ReturnBucketRow {
            label: format!("{lo:.1}% to {hi:.1}%"),
            min_pct: Some(lo),
            max_pct: Some(hi),
            count,
            pct_of_total,
            percentile_range: format!("{start_cum}% - {end_cum}%"),
            probability_label: format!(
                "{:.1}% - {:.1}%",
                (hundred - end_cum).max(Decimal::ZERO),
                (hundred - start_cum).max(Decimal::ZERO)
            ),
            price_range_low: Some(price_from_return(ref_price, lo)),
            price_range_high: Some(price_from_return(ref_price, hi)),
        });
    }

    // Probability-Based Levels (`10%..99%` chance spot closes ABOVE implied level)
    let prob_cuts = [10usize, 20, 30, 40, 50, 60, 70, 80, 90, 95, 99];
    let probability_levels = prob_cuts
        .iter()
        .map(|&prob_above| {
            let quantile_p = Decimal::from((100 - prob_above) as i64);
            let r_cut = empirical_quantile(&sorted_asc, quantile_p);
            ProbabilityLevelRow {
                prob_above_pct: prob_above,
                return_pct: r_cut,
                implied_price: price_from_return(ref_price, r_cut).round_dp(0),
                change_pct: r_cut,
                scenario: scenario_tag(r_cut).to_string(),
            }
        })
        .collect();

    // Green vs Red comparison
    let greens: Vec<Decimal> = periods
        .iter()
        .filter(|p| p.return_pct > Decimal::ZERO)
        .map(|p| p.return_pct)
        .collect();
    let reds: Vec<Decimal> = periods
        .iter()
        .filter(|p| p.return_pct < Decimal::ZERO)
        .map(|p| p.return_pct)
        .collect();

    let (green_mean, green_std) = mean_and_sample_std(&greens);
    let (red_mean, red_std) = mean_and_sample_std(&reds);
    let green_stats = PolarityStats {
        count: greens.len(),
        share_pct: ((Decimal::from(greens.len() as i64) / total_dec) * hundred).round_dp(1),
        mean_pct: green_mean.round_dp(2),
        median_pct: median_of(&greens),
        std_dev_pct: green_std.round_dp(2),
        extreme_pct: greens.iter().copied().max().unwrap_or(Decimal::ZERO),
    };
    let red_stats = PolarityStats {
        count: reds.len(),
        share_pct: ((Decimal::from(reds.len() as i64) / total_dec) * hundred).round_dp(1),
        mean_pct: red_mean.round_dp(2),
        median_pct: median_of(&reds),
        std_dev_pct: red_std.round_dp(2),
        extreme_pct: reds.iter().copied().min().unwrap_or(Decimal::ZERO),
    };

    let mag_bands = [
        ("0% - 1%", Decimal::ZERO, Some(Decimal::from(1))),
        ("1% - 2%", Decimal::from(1), Some(Decimal::from(2))),
        ("2% - 3%", Decimal::from(2), Some(Decimal::from(3))),
        ("3% - 4%", Decimal::from(3), Some(Decimal::from(4))),
        ("4% - 5%", Decimal::from(4), Some(Decimal::from(5))),
        ("> 5%", Decimal::from(5), None),
    ];
    let green_dec = Decimal::from(greens.len().max(1) as i64);
    let red_dec = Decimal::from(reds.len().max(1) as i64);
    let magnitude_buckets = mag_bands
        .iter()
        .map(|(label, lo, hi)| {
            let gc = greens
                .iter()
                .filter(|&&r| match hi {
                    Some(h) => r > *lo && r <= *h,
                    None => r > *lo,
                })
                .count();
            let rc = reds
                .iter()
                .filter(|&&r| {
                    let a = r.abs();
                    match hi {
                        Some(h) => a > *lo && a <= *h,
                        None => a > *lo,
                    }
                })
                .count();
            MagnitudePdfBucket {
                label: (*label).to_string(),
                green_count: gc,
                green_pct: ((Decimal::from(gc as i64) / green_dec) * hundred).round_dp(1),
                red_count: rc,
                red_pct: ((Decimal::from(rc as i64) / red_dec) * hundred).round_dp(1),
            }
        })
        .collect();

    let green_vs_red = GreenVsRedComparison {
        green: green_stats,
        red: red_stats,
        magnitude_buckets,
    };

    // Conditional forward after large move
    let thresh = if tail_threshold_pct > Decimal::ZERO {
        tail_threshold_pct
    } else {
        Decimal::from(2)
    };
    let mut after_pos_events = Vec::new();
    let mut after_neg_events = Vec::new();
    for w in periods.windows(2) {
        if w[0].return_pct >= thresh {
            after_pos_events.push(ForwardConditionEvent {
                trigger_end_date: w[0].end_date,
                trigger_return_pct: w[0].return_pct,
                next_end_date: w[1].end_date,
                next_return_pct: w[1].return_pct,
            });
        } else if w[0].return_pct <= -thresh {
            after_neg_events.push(ForwardConditionEvent {
                trigger_end_date: w[0].end_date,
                trigger_return_pct: w[0].return_pct,
                next_end_date: w[1].end_date,
                next_return_pct: w[1].return_pct,
            });
        }
    }

    let after_pos_rets: Vec<Decimal> = after_pos_events.iter().map(|e| e.next_return_pct).collect();
    let after_neg_rets: Vec<Decimal> = after_neg_events.iter().map(|e| e.next_return_pct).collect();
    let (pos_fwd_mean, _) = mean_and_sample_std(&after_pos_rets);
    let (neg_fwd_mean, _) = mean_and_sample_std(&after_neg_rets);
    let pos_fwd_win = if after_pos_rets.is_empty() {
        Decimal::ZERO
    } else {
        let w = after_pos_rets
            .iter()
            .filter(|&&r| r > Decimal::ZERO)
            .count();
        ((Decimal::from(w as i64) / Decimal::from(after_pos_rets.len() as i64)) * hundred)
            .round_dp(1)
    };
    let neg_fwd_win = if after_neg_rets.is_empty() {
        Decimal::ZERO
    } else {
        let w = after_neg_rets
            .iter()
            .filter(|&&r| r > Decimal::ZERO)
            .count();
        ((Decimal::from(w as i64) / Decimal::from(after_neg_rets.len() as i64)) * hundred)
            .round_dp(1)
    };

    let forward_conditional = ForwardConditionalGroup {
        threshold_pct: thresh,
        after_large_pos_mean_pct: pos_fwd_mean.round_dp(2),
        after_large_pos_median_pct: median_of(&after_pos_rets),
        after_large_pos_win_pct: pos_fwd_win,
        after_large_pos_events: after_pos_events,
        after_large_neg_mean_pct: neg_fwd_mean.round_dp(2),
        after_large_neg_median_pct: median_of(&after_neg_rets),
        after_large_neg_win_pct: neg_fwd_win,
        after_large_neg_events: after_neg_events,
    };

    // Survivability Outliers
    let cov = coverage_pct.max(Decimal::from(50)).min(Decimal::from(99));
    let tail_each_side = ((hundred - cov) / Decimal::from(2)).round_dp(4);
    let lower_bound = empirical_quantile(&sorted_asc, tail_each_side);
    let upper_bound = empirical_quantile(&sorted_asc, hundred - tail_each_side);

    let mut scatter_points = Vec::with_capacity(periods.len());
    let mut outlier_table = Vec::new();
    let mut upside_count = 0usize;
    let mut downside_count = 0usize;

    for p in periods {
        let status = if p.return_pct > upper_bound {
            upside_count += 1;
            OutlierStatus::UpsideOutlier
        } else if p.return_pct < lower_bound {
            downside_count += 1;
            OutlierStatus::DownsideOutlier
        } else {
            OutlierStatus::WithinBoundary
        };
        let point = OutlierPeriodPoint {
            index: p.index,
            start_date: p.start_date,
            end_date: p.end_date,
            return_pct: p.return_pct,
            status,
        };
        if status != OutlierStatus::WithinBoundary {
            outlier_table.push(point.clone());
        }
        scatter_points.push(point);
    }

    outlier_table.sort_by_key(|p| std::cmp::Reverse(p.return_pct.abs()));
    let total_outliers = upside_count + downside_count;
    let outlier_pct = ((Decimal::from(total_outliers as i64) / total_dec) * hundred).round_dp(1);

    let outliers = OutlierAnalysis {
        kpis: OutlierKpis {
            coverage_pct: cov,
            upper_bound_pct: upper_bound,
            lower_bound_pct: lower_bound,
            total_periods: total,
            total_outliers,
            outlier_pct,
            upside_outliers: upside_count,
            downside_outliers: downside_count,
        },
        scatter_points,
        outlier_table,
    };

    DistributionBundle {
        percentiles,
        current_price: spot,
        current_reference_price: ref_price,
        current_reference_change_pct: ref_change_pct,
        coarse_buckets,
        granular_10bps,
        probability_levels,
        green_vs_red,
        forward_conditional,
        outliers,
    }
}
