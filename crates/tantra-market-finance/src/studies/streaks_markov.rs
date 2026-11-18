//! Directional streak analyzer and 6-state streak-length Markov transition chain.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::returns::mean_and_sample_std;
use super::schedule::SlicedPeriod;

/// Directional polarity of a streak (`r == 0` periods are excluded cleanly).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreakDirection {
    /// Positive return run (`r > 0`).
    Positive,
    /// Negative return run (`r < 0`).
    Negative,
}

/// Six discrete Markov states encoding streak direction × duration bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkovState {
    /// 1–2 consecutive positive periods.
    ShortPositive,
    /// 3–4 consecutive positive periods.
    MediumPositive,
    /// 5+ consecutive positive periods.
    LongPositive,
    /// 1–2 consecutive negative periods.
    ShortNegative,
    /// 3–4 consecutive negative periods.
    MediumNegative,
    /// 5+ consecutive negative periods.
    LongNegative,
}

impl MarkovState {
    /// Fixed canonical ordering matching the 6×6 UI heatmap matrix.
    pub const ALL: [MarkovState; 6] = [
        MarkovState::ShortPositive,
        MarkovState::MediumPositive,
        MarkovState::LongPositive,
        MarkovState::ShortNegative,
        MarkovState::MediumNegative,
        MarkovState::LongNegative,
    ];

    /// Maps a streak polarity and run length to its Markov state.
    pub fn from_streak(dir: StreakDirection, len: usize) -> Self {
        match (dir, len) {
            (StreakDirection::Positive, 0..=2) => MarkovState::ShortPositive,
            (StreakDirection::Positive, 3..=4) => MarkovState::MediumPositive,
            (StreakDirection::Positive, _) => MarkovState::LongPositive,
            (StreakDirection::Negative, 0..=2) => MarkovState::ShortNegative,
            (StreakDirection::Negative, 3..=4) => MarkovState::MediumNegative,
            (StreakDirection::Negative, _) => MarkovState::LongNegative,
        }
    }

    /// Display label for tooltips and matrix headers.
    pub fn label(self) -> &'static str {
        match self {
            MarkovState::ShortPositive => "Short Pos (1-2)",
            MarkovState::MediumPositive => "Med Pos (3-4)",
            MarkovState::LongPositive => "Long Pos (5+)",
            MarkovState::ShortNegative => "Short Neg (1-2)",
            MarkovState::MediumNegative => "Med Neg (3-4)",
            MarkovState::LongNegative => "Long Neg (5+)",
        }
    }

    /// Returns `true` when the state represents a positive run.
    pub fn is_positive(self) -> bool {
        matches!(
            self,
            MarkovState::ShortPositive | MarkovState::MediumPositive | MarkovState::LongPositive
        )
    }
}

/// One completed (or currently active) contiguous directional run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreakDetail {
    /// 1-based chronological streak index.
    pub index: usize,
    /// Streak direction.
    pub streak_type: StreakDirection,
    /// Number of non-neutral periods in this run.
    pub length: usize,
    /// First period start date.
    pub start_date: NaiveDate,
    /// Final period end date.
    pub end_date: NaiveDate,
    /// Opening price of first period in streak.
    pub start_price: Decimal,
    /// Closing price of final period in streak.
    pub end_price: Decimal,
    /// Compounded total return over the streak (%).
    pub total_return_pct: Decimal,
    /// Average single-period return inside the streak (%).
    pub avg_period_return_pct: Decimal,
}

/// Aggregate KPI metrics across all completed streaks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StreakSummary {
    /// Currently active streak direction (`None` if zero non-neutral periods).
    pub current_streak_type: Option<StreakDirection>,
    /// Length of the active streak.
    pub current_streak_length: usize,
    /// Compounded return of the active streak (%).
    pub current_streak_return_pct: Decimal,
    /// Markov state of the active streak.
    pub current_markov_state: Option<MarkovState>,
    /// Total positive streaks recorded.
    pub positive_streak_count: usize,
    /// Total negative streaks recorded.
    pub negative_streak_count: usize,
    /// Longest positive streak length.
    pub longest_positive: usize,
    /// Longest negative streak length.
    pub longest_negative: usize,
    /// Best compounded return of any positive streak (%).
    pub best_positive_return_pct: Decimal,
    /// Worst compounded return of any negative streak (%).
    pub worst_negative_return_pct: Decimal,
    /// Mean length of positive streaks.
    pub avg_positive_length: Decimal,
    /// Mean length of negative streaks.
    pub avg_negative_length: Decimal,
    /// Exact zero-return periods excluded before building streaks.
    pub excluded_neutral_count: usize,
}

/// Histogram entry comparing positive vs negative streak frequencies at a given run length.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreakLengthBucket {
    /// Run length (`1, 2, 3, ...`).
    pub length: usize,
    /// Number of positive streaks with this length.
    pub positive_count: usize,
    /// Number of negative streaks with this length.
    pub negative_count: usize,
}

/// Distribution summary statistics for one side (positive or negative streaks).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SideStreakStats {
    /// Most frequent streak length (mode).
    pub most_common_length: usize,
    /// Count observed at the mode.
    pub most_common_count: usize,
    /// Median streak length.
    pub median_length: Decimal,
    /// Sample standard deviation of streak lengths.
    pub std_dev_length: Decimal,
}

/// Complete Streak Analysis bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StreakAnalysis {
    /// High-level streak KPIs + active streak banner data.
    pub summary: StreakSummary,
    /// Chronological list of streaks (timeline + data table).
    pub details: Vec<StreakDetail>,
    /// Length histogram (`1..max_len`).
    pub length_distribution: Vec<StreakLengthBucket>,
    /// Descriptive statistics for positive streak lengths.
    pub positive_stats: SideStreakStats,
    /// Descriptive statistics for negative streak lengths.
    pub negative_stats: SideStreakStats,
}

/// Probability weight (`0..100`) associated with a single Markov state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkovStateProbability {
    /// Target state.
    pub state: MarkovState,
    /// Display label.
    pub label: String,
    /// Empirical probability percentage (`0.00..=100.00`).
    pub probability_pct: Decimal,
}

/// One row of the 6×6 Markov transition probability matrix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkovTransitionRow {
    /// Source state ("Current State").
    pub from_state: MarkovState,
    /// Human label for `from_state`.
    pub from_label: String,
    /// Observed outgoing transition count from this state.
    pub observation_count: usize,
    /// Probability distribution across `MarkovState::ALL` (`0..=100` %).
    pub probs_pct: Vec<Decimal>,
}

/// Conditional next-streak forecast anchored to the current active streak state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MarkovCurrentPrediction {
    /// Currently active streak state.
    pub current_state: Option<MarkovState>,
    /// Current state label.
    pub current_label: String,
    /// Total probability (%) of transitioning/staying in a Positive streak state next.
    pub positive_next_pct: Decimal,
    /// Total probability (%) of transitioning to a Negative streak state next.
    pub negative_next_pct: Decimal,
    /// Single most likely next Markov state.
    pub strongest_next_state: Option<MarkovState>,
    /// Label of `strongest_next_state`.
    pub strongest_next_label: String,
    /// Empirical probability (%) of `strongest_next_state`.
    pub strongest_probability_pct: Decimal,
    /// Full ranked breakdown across all 6 target states.
    pub state_breakdown: Vec<MarkovStateProbability>,
}

/// Complete 6-State Streak Markov Chain analysis bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MarkovAnalysis {
    /// 6×6 Transition Probability Matrix rows (matching `MarkovState::ALL`).
    pub transition_matrix: Vec<MarkovTransitionRow>,
    /// Conditional forecast from active streak state.
    pub current_prediction: MarkovCurrentPrediction,
    /// Equilibrium stationary distribution across the 6 states (%).
    pub steady_state: Vec<MarkovStateProbability>,
}

fn compute_side_stats(lengths: &[usize]) -> SideStreakStats {
    if lengths.is_empty() {
        return SideStreakStats::default();
    }
    let mut freq: BTreeMap<usize, usize> = BTreeMap::new();
    for &l in lengths {
        *freq.entry(l).or_insert(0) += 1;
    }
    let (&mode_len, &mode_count) = freq
        .iter()
        .max_by_key(|(len, count)| (*count, std::cmp::Reverse(*len)))
        .unwrap();

    let mut sorted = lengths.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    let median = if sorted.len() % 2 == 1 {
        Decimal::from(sorted[mid] as i64)
    } else {
        ((Decimal::from(sorted[mid - 1] as i64) + Decimal::from(sorted[mid] as i64))
            / Decimal::from(2))
        .round_dp(2)
    };

    let decs: Vec<Decimal> = lengths.iter().map(|&l| Decimal::from(l as i64)).collect();
    let (_, std_dev) = mean_and_sample_std(&decs);

    SideStreakStats {
        most_common_length: mode_len,
        most_common_count: mode_count,
        median_length: median,
        std_dev_length: std_dev.round_dp(2),
    }
}

/// Analyzes streaks and derives the empirical 6×6 streak-based Markov chain.
pub fn analyze_streaks_and_markov(periods: &[SlicedPeriod]) -> (StreakAnalysis, MarkovAnalysis) {
    let hundred = Decimal::from(100);
    let mut excluded_neutral_count = 0usize;
    let mut non_neutral: Vec<&SlicedPeriod> = Vec::with_capacity(periods.len());

    for p in periods {
        if p.return_pct.is_zero() {
            excluded_neutral_count += 1;
        } else {
            non_neutral.push(p);
        }
    }

    if non_neutral.is_empty() {
        let summary = StreakSummary {
            excluded_neutral_count,
            ..Default::default()
        };
        return (
            StreakAnalysis {
                summary,
                ..Default::default()
            },
            MarkovAnalysis::default(),
        );
    }

    let mut details: Vec<StreakDetail> = Vec::new();
    let mut run_dir = if non_neutral[0].return_pct > Decimal::ZERO {
        StreakDirection::Positive
    } else {
        StreakDirection::Negative
    };
    let mut run_start = 0usize;

    let finalize_run =
        |idx_num: usize, dir: StreakDirection, slice: &[&SlicedPeriod]| -> StreakDetail {
            let first = slice[0];
            let last = slice[slice.len() - 1];
            let mut factor = Decimal::ONE;
            let mut sum_ret = Decimal::ZERO;
            for &item in slice {
                let r_frac = (item.return_pct / hundred).round_dp(8);
                factor = (factor * (Decimal::ONE + r_frac)).round_dp(8);
                sum_ret += item.return_pct;
            }
            let compounded_pct = ((factor - Decimal::ONE) * hundred).round_dp(2);
            let avg_pct = (sum_ret / Decimal::from(slice.len() as i64)).round_dp(2);
            StreakDetail {
                index: idx_num,
                streak_type: dir,
                length: slice.len(),
                start_date: first.start_date,
                end_date: last.end_date,
                start_price: first.entry_price,
                end_price: last.exit_price,
                total_return_pct: compounded_pct,
                avg_period_return_pct: avg_pct,
            }
        };

    for i in 1..non_neutral.len() {
        let dir = if non_neutral[i].return_pct > Decimal::ZERO {
            StreakDirection::Positive
        } else {
            StreakDirection::Negative
        };
        if dir != run_dir {
            let detail = finalize_run(details.len() + 1, run_dir, &non_neutral[run_start..i]);
            details.push(detail);
            run_dir = dir;
            run_start = i;
        }
    }
    let last_detail = finalize_run(details.len() + 1, run_dir, &non_neutral[run_start..]);
    details.push(last_detail);

    // Aggregate StreakSummary & Length Distribution
    let mut pos_lengths = Vec::new();
    let mut neg_lengths = Vec::new();
    let mut longest_pos = 0usize;
    let mut longest_neg = 0usize;
    let mut best_pos = Decimal::ZERO;
    let mut worst_neg = Decimal::ZERO;
    let mut has_pos = false;
    let mut has_neg = false;
    let mut hist: BTreeMap<usize, (usize, usize)> = BTreeMap::new();

    for d in &details {
        let entry = hist.entry(d.length).or_insert((0, 0));
        match d.streak_type {
            StreakDirection::Positive => {
                pos_lengths.push(d.length);
                entry.0 += 1;
                if d.length > longest_pos {
                    longest_pos = d.length;
                }
                if !has_pos || d.total_return_pct > best_pos {
                    best_pos = d.total_return_pct;
                    has_pos = true;
                }
            }
            StreakDirection::Negative => {
                neg_lengths.push(d.length);
                entry.1 += 1;
                if d.length > longest_neg {
                    longest_neg = d.length;
                }
                if !has_neg || d.total_return_pct < worst_neg {
                    worst_neg = d.total_return_pct;
                    has_neg = true;
                }
            }
        }
    }

    let avg_pos_len = if pos_lengths.is_empty() {
        Decimal::ZERO
    } else {
        let s: usize = pos_lengths.iter().sum();
        (Decimal::from(s as i64) / Decimal::from(pos_lengths.len() as i64)).round_dp(2)
    };
    let avg_neg_len = if neg_lengths.is_empty() {
        Decimal::ZERO
    } else {
        let s: usize = neg_lengths.iter().sum();
        (Decimal::from(s as i64) / Decimal::from(neg_lengths.len() as i64)).round_dp(2)
    };

    let active = details.last().unwrap();
    let active_state = MarkovState::from_streak(active.streak_type, active.length);

    let summary = StreakSummary {
        current_streak_type: Some(active.streak_type),
        current_streak_length: active.length,
        current_streak_return_pct: active.total_return_pct,
        current_markov_state: Some(active_state),
        positive_streak_count: pos_lengths.len(),
        negative_streak_count: neg_lengths.len(),
        longest_positive: longest_pos,
        longest_negative: longest_neg,
        best_positive_return_pct: best_pos,
        worst_negative_return_pct: worst_neg,
        avg_positive_length: avg_pos_len,
        avg_negative_length: avg_neg_len,
        excluded_neutral_count,
    };

    let length_distribution: Vec<StreakLengthBucket> = hist
        .into_iter()
        .map(
            |(length, (positive_count, negative_count))| StreakLengthBucket {
                length,
                positive_count,
                negative_count,
            },
        )
        .collect();

    let streak_analysis = StreakAnalysis {
        summary,
        details: details.clone(),
        length_distribution,
        positive_stats: compute_side_stats(&pos_lengths),
        negative_stats: compute_side_stats(&neg_lengths),
    };

    // Build 6×6 Markov transition matrix across completed consecutive streaks
    let state_idx = |s: MarkovState| -> usize {
        match s {
            MarkovState::ShortPositive => 0,
            MarkovState::MediumPositive => 1,
            MarkovState::LongPositive => 2,
            MarkovState::ShortNegative => 3,
            MarkovState::MediumNegative => 4,
            MarkovState::LongNegative => 5,
        }
    };

    let mut counts = [[0usize; 6]; 6];
    let mut row_totals = [0usize; 6];
    for w in details.windows(2) {
        let from_s = MarkovState::from_streak(w[0].streak_type, w[0].length);
        let to_s = MarkovState::from_streak(w[1].streak_type, w[1].length);
        let r = state_idx(from_s);
        let c = state_idx(to_s);
        counts[r][c] += 1;
        row_totals[r] += 1;
    }

    let mut probs_unit = [[Decimal::ZERO; 6]; 6];
    let mut transition_matrix = Vec::with_capacity(6);
    for (r, &from_state) in MarkovState::ALL.iter().enumerate() {
        let total = row_totals[r];
        let mut probs_pct = Vec::with_capacity(6);
        for c in 0..6 {
            let p_unit = if total == 0 {
                Decimal::ZERO
            } else {
                (Decimal::from(counts[r][c] as i64) / Decimal::from(total as i64)).round_dp(8)
            };
            probs_unit[r][c] = p_unit;
            probs_pct.push((p_unit * hundred).round_dp(2));
        }
        transition_matrix.push(MarkovTransitionRow {
            from_state,
            from_label: from_state.label().to_string(),
            observation_count: total,
            probs_pct,
        });
    }

    // Current prediction
    let active_r = state_idx(active_state);
    let mut positive_next_pct = Decimal::ZERO;
    let mut negative_next_pct = Decimal::ZERO;
    let mut state_breakdown: Vec<MarkovStateProbability> = MarkovState::ALL
        .iter()
        .enumerate()
        .map(|(c, &s)| {
            let p = transition_matrix[active_r].probs_pct[c];
            if s.is_positive() {
                positive_next_pct += p;
            } else {
                negative_next_pct += p;
            }
            MarkovStateProbability {
                state: s,
                label: s.label().to_string(),
                probability_pct: p,
            }
        })
        .collect();
    state_breakdown.sort_by_key(|x| std::cmp::Reverse(x.probability_pct));
    let strongest = state_breakdown.first().cloned();

    let current_prediction = MarkovCurrentPrediction {
        current_state: Some(active_state),
        current_label: active_state.label().to_string(),
        positive_next_pct: positive_next_pct.round_dp(2),
        negative_next_pct: negative_next_pct.round_dp(2),
        strongest_next_state: strongest.as_ref().map(|x| x.state),
        strongest_next_label: strongest
            .as_ref()
            .map(|x| x.label.clone())
            .unwrap_or_default(),
        strongest_probability_pct: strongest
            .as_ref()
            .map(|x| x.probability_pct)
            .unwrap_or(Decimal::ZERO),
        state_breakdown,
    };

    // Steady-state eigenvector via power iteration (pi = pi * P)
    // Rows with zero outgoing transitions fall back to a uniform distribution so Markov
    // chain stays stochastic cleanly.
    let uniform = (Decimal::ONE / Decimal::from(6)).round_dp(8);
    let mut stoch_p = probs_unit;
    for r in 0..6 {
        if row_totals[r] == 0 {
            stoch_p[r] = [uniform; 6];
        }
    }

    let mut pi = [uniform; 6];
    let tol = Decimal::new(1, 7); // 1e-7
    for _ in 0..160 {
        let mut next = [Decimal::ZERO; 6];
        for c in 0..6 {
            let mut acc = Decimal::ZERO;
            for r in 0..6 {
                acc += (pi[r] * stoch_p[r][c]).round_dp(10);
            }
            next[c] = acc.round_dp(8);
        }
        // Renormalize to guard against cumulative rounding drift
        let sum: Decimal = next.iter().copied().sum();
        if sum > Decimal::ZERO {
            for val in &mut next {
                *val = (*val / sum).round_dp(8);
            }
        }
        let l1_diff: Decimal = (0..6).map(|i| (next[i] - pi[i]).abs()).sum();
        pi = next;
        if l1_diff < tol {
            break;
        }
    }

    let steady_state = MarkovState::ALL
        .iter()
        .enumerate()
        .map(|(i, &state)| MarkovStateProbability {
            state,
            label: state.label().to_string(),
            probability_pct: (pi[i] * hundred).round_dp(2),
        })
        .collect();

    let markov_analysis = MarkovAnalysis {
        transition_matrix,
        current_prediction,
        steady_state,
    };

    (streak_analysis, markov_analysis)
}
