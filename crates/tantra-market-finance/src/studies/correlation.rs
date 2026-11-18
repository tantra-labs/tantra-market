//! Pure `Decimal` Cross-Asset Correlation (`N × N` Pearson & Spearman),
//! Rolling Pair Beta, and Spot / Near–Next Spread tracking (`wasm32`-ready).

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::decimal_sqrt;
use super::schedule::OhlcvBar;

/// One rolling correlation & spread sample between primary and comparison benchmark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollingPairPoint {
    /// Trading date.
    pub date: NaiveDate,
    /// Trailing `rolling_window` Pearson correlation (`-1..=1`).
    pub rolling_corr: Decimal,
    /// Relative normalized spread `(Primary_idx - Benchmark_idx)` in percentage points.
    pub relative_spread_pct: Decimal,
    /// Raw price spread `(Primary_close - Benchmark_close)`.
    pub raw_spread: Decimal,
}

/// Complete Tab 7 cross-asset correlation & pair spread bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CorrelationAnalysis {
    /// Labels corresponding to matrix rows & columns.
    pub labels: Vec<String>,
    /// Primary instrument id.
    pub primary_id: String,
    /// Comparison benchmark id for rolling correlation & pair spread.
    pub benchmark_id: String,
    /// Rolling window length used (in overlapping trading sessions).
    pub rolling_window: usize,
    /// Number of aligned trading sessions between primary and benchmark.
    pub aligned_sessions: usize,
    /// `N × N` Pearson correlation matrix (each entry rounded to 4 dp).
    pub pearson_matrix: Vec<Vec<Decimal>>,
    /// `N × N` Spearman rank correlation matrix (each entry rounded to 4 dp).
    pub spearman_matrix: Vec<Vec<Decimal>>,
    /// Empirical OLS Beta of `primary` with respect to `benchmark` (`Cov(P,B) / Var(B)`).
    pub pair_beta: Decimal,
    /// Full-sample Pearson correlation between `primary` and `benchmark`.
    pub pair_pearson: Decimal,
    /// Full-sample Spearman correlation between `primary` and `benchmark`.
    pub pair_spearman: Decimal,
    /// Latest raw spread (`primary - benchmark`).
    pub latest_raw_spread: Decimal,
    /// Latest normalized spread (`%` outperformance since start of aligned window).
    pub latest_relative_spread_pct: Decimal,
    /// Rolling correlation and spread curve.
    pub rolling_series: Vec<RollingPairPoint>,
}

fn daily_returns_map(
    bars: &[OhlcvBar],
) -> (BTreeMap<NaiveDate, Decimal>, BTreeMap<NaiveDate, Decimal>) {
    let mut closes = BTreeMap::new();
    let mut rets = BTreeMap::new();
    let hundred = Decimal::from(100);
    for (i, bar) in bars.iter().enumerate() {
        closes.insert(bar.date, bar.close);
        if i > 0 {
            let prev = bars[i - 1].close;
            if prev > Decimal::ZERO {
                let r = (((bar.close - prev) / prev) * hundred).round_dp(6);
                rets.insert(bar.date, r);
            }
        }
    }
    (closes, rets)
}

fn pearson_and_beta(xs: &[Decimal], ys: &[Decimal]) -> (Decimal, Decimal) {
    let n = xs.len().min(ys.len());
    if n < 2 {
        return (Decimal::ZERO, Decimal::ZERO);
    }
    let n_dec = Decimal::from(n as i64);
    let mean_x = xs[..n].iter().copied().sum::<Decimal>() / n_dec;
    let mean_y = ys[..n].iter().copied().sum::<Decimal>() / n_dec;

    let mut cov = Decimal::ZERO;
    let mut var_x = Decimal::ZERO;
    let mut var_y = Decimal::ZERO;

    for i in 0..n {
        let dx = xs[i] - mean_x;
        let dy = ys[i] - mean_y;
        cov += (dx * dy).round_dp(10);
        var_x += (dx * dx).round_dp(10);
        var_y += (dy * dy).round_dp(10);
    }

    if var_x <= Decimal::ZERO || var_y <= Decimal::ZERO {
        return (Decimal::ZERO, Decimal::ZERO);
    }
    let denom = decimal_sqrt((var_x * var_y).round_dp(10));
    let corr = if denom > Decimal::ZERO {
        (cov / denom)
            .max(Decimal::NEGATIVE_ONE)
            .min(Decimal::ONE)
            .round_dp(4)
    } else {
        Decimal::ZERO
    };
    let beta = (cov / var_y).round_dp(4);
    (corr, beta)
}

fn average_ranks(vals: &[Decimal]) -> Vec<Decimal> {
    let n = vals.len();
    let mut indexed: Vec<(usize, Decimal)> = vals.iter().copied().enumerate().collect();
    indexed.sort_by_key(|a| a.1);

    let mut ranks = vec![Decimal::ZERO; n];
    let mut i = 0usize;
    let two = Decimal::from(2);
    while i < n {
        let mut j = i + 1;
        while j < n && indexed[j].1 == indexed[i].1 {
            j += 1;
        }
        // Average rank across 1-based tie range (i+1 ..= j)
        let avg_rank = Decimal::from((i + 1 + j) as i64) / two;
        for item in &indexed[i..j] {
            ranks[item.0] = avg_rank;
        }
        i = j;
    }
    ranks
}

fn spearman_corr(xs: &[Decimal], ys: &[Decimal]) -> Decimal {
    if xs.len() < 3 || xs.len() != ys.len() {
        return Decimal::ZERO;
    }
    let rx = average_ranks(xs);
    let ry = average_ranks(ys);
    pearson_and_beta(&rx, &ry).0
}

/// Evaluates `N × N` Pearson & Spearman correlation matrices, pair beta, and rolling spread.
pub fn analyze_cross_asset_correlation(
    series: &[(String, Vec<OhlcvBar>)],
    primary_id: &str,
    benchmark_id: &str,
    rolling_window: usize,
) -> CorrelationAnalysis {
    let n = series.len();
    if n == 0 {
        return CorrelationAnalysis::default();
    }

    let labels: Vec<String> = series.iter().map(|(id, _)| id.clone()).collect();
    let mut close_maps = Vec::with_capacity(n);
    let mut ret_maps = Vec::with_capacity(n);

    for (_id, bars) in series {
        let (c, r) = daily_returns_map(bars);
        close_maps.push(c);
        ret_maps.push(r);
    }

    let mut pearson_matrix = vec![vec![Decimal::ZERO; n]; n];
    let mut spearman_matrix = vec![vec![Decimal::ZERO; n]; n];

    for i in 0..n {
        pearson_matrix[i][i] = Decimal::ONE;
        spearman_matrix[i][i] = Decimal::ONE;
        for j in (i + 1)..n {
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (d, rx) in &ret_maps[i] {
                if let Some(ry) = ret_maps[j].get(d) {
                    xs.push(*rx);
                    ys.push(*ry);
                }
            }
            let (p, _) = pearson_and_beta(&xs, &ys);
            let s = spearman_corr(&xs, &ys);
            pearson_matrix[i][j] = p;
            pearson_matrix[j][i] = p;
            spearman_matrix[i][j] = s;
            spearman_matrix[j][i] = s;
        }
    }

    let p_idx = labels.iter().position(|l| l == primary_id).unwrap_or(0);
    let b_idx = labels
        .iter()
        .position(|l| l == benchmark_id)
        .unwrap_or(if n > 1 { 1 } else { 0 });

    let mut aligned_dates = Vec::new();
    for d in close_maps[p_idx].keys() {
        if close_maps[b_idx].contains_key(d) {
            aligned_dates.push(*d);
        }
    }

    let mut ret_p = Vec::new();
    let mut ret_b = Vec::new();
    let mut ret_dates = Vec::new();
    for d in &aligned_dates {
        if let (Some(rp), Some(rb)) = (ret_maps[p_idx].get(d), ret_maps[b_idx].get(d)) {
            ret_p.push(*rp);
            ret_b.push(*rb);
            ret_dates.push(*d);
        }
    }

    let (pair_pearson, pair_beta) = pearson_and_beta(&ret_p, &ret_b);
    let pair_spearman = spearman_corr(&ret_p, &ret_b);

    let win = rolling_window.clamp(5, 250);
    let hundred = Decimal::from(100);
    let mut ret_idx_map: HashMap<NaiveDate, usize> = HashMap::new();
    for (idx, d) in ret_dates.iter().enumerate() {
        ret_idx_map.insert(*d, idx);
    }

    let base_p = aligned_dates
        .first()
        .and_then(|d| close_maps[p_idx].get(d).copied())
        .unwrap_or(Decimal::ONE);
    let base_b = aligned_dates
        .first()
        .and_then(|d| close_maps[b_idx].get(d).copied())
        .unwrap_or(Decimal::ONE);

    let mut rolling_series = Vec::new();
    let mut latest_raw_spread = Decimal::ZERO;
    let mut latest_rel_spread = Decimal::ZERO;

    for d in &aligned_dates {
        let cp = close_maps[p_idx][d];
        let cb = close_maps[b_idx][d];
        let raw_spread = (cp - cb).round_dp(4);
        let rel_spread = if base_p > Decimal::ZERO && base_b > Decimal::ZERO {
            (((cp / base_p) - (cb / base_b)) * hundred).round_dp(2)
        } else {
            Decimal::ZERO
        };
        latest_raw_spread = raw_spread;
        latest_rel_spread = rel_spread;

        if let Some(&r_idx) = ret_idx_map.get(d)
            && r_idx + 1 >= win
        {
            let start = r_idx + 1 - win;
            let (rc, _) = pearson_and_beta(&ret_p[start..=r_idx], &ret_b[start..=r_idx]);
            rolling_series.push(RollingPairPoint {
                date: *d,
                rolling_corr: rc,
                relative_spread_pct: rel_spread,
                raw_spread,
            });
        }
    }

    let primary_label = labels[p_idx].clone();
    let benchmark_label = labels[b_idx].clone();

    CorrelationAnalysis {
        labels,
        primary_id: primary_label,
        benchmark_id: benchmark_label,
        rolling_window: win,
        aligned_sessions: aligned_dates.len(),
        pearson_matrix,
        spearman_matrix,
        pair_beta,
        pair_pearson,
        pair_spearman,
        latest_raw_spread,
        latest_relative_spread_pct: latest_rel_spread,
        rolling_series,
    }
}
