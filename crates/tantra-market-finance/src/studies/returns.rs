//! Core period-return KPIs, moving average trajectory, and Daily OHLC summary metrics.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::decimal_sqrt;
use super::schedule::{OhlcvBar, SlicedPeriod};

/// 3×3 statistical KPI summary for the Returns Analysis panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReturnKpis {
    /// Arithmetic mean of period returns (%).
    pub avg_return_pct: Decimal,
    /// Maximum single-period return (%).
    pub max_gain_pct: Decimal,
    /// Minimum single-period return (%).
    pub max_loss_pct: Decimal,
    /// Sample standard deviation of period returns (%).
    pub std_dev_pct: Decimal,
    /// Count of strictly positive return periods (`r > 0`).
    pub positive_count: usize,
    /// Percentage of total periods that were positive.
    pub positive_pct: Decimal,
    /// Count of strictly negative return periods (`r < 0`).
    pub negative_count: usize,
    /// Percentage of total periods that were negative.
    pub negative_pct: Decimal,
    /// Count of exact zero return periods (`r == 0`).
    pub neutral_count: usize,
    /// Empirical period Sharpe ratio (`avg_return / std_dev`).
    pub sharpe_ratio: Decimal,
    /// Empirical Sortino ratio (`avg_return / downside_deviation`).
    pub sortino_ratio: Decimal,
    /// Profit factor (`sum(r > 0) / abs(sum(r < 0))`).
    pub profit_factor: Decimal,
}

/// One point in the interactive Period Return Trajectory + SMA chart & data table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodPoint {
    /// 1-based period number.
    pub index: usize,
    /// Cycle start trading date.
    pub start_date: NaiveDate,
    /// Cycle end trading date.
    pub end_date: NaiveDate,
    /// Cycle entry/open price.
    pub entry_price: Decimal,
    /// Cycle exit/close price.
    pub exit_price: Decimal,
    /// Cycle high price.
    pub high_price: Decimal,
    /// Cycle low price.
    pub low_price: Decimal,
    /// Period percentage return.
    pub return_pct: Decimal,
    /// Rolling SMA of `return_pct` over `ma_window` periods (`None` while warming up).
    pub ma_pct: Option<Decimal>,
    /// Calendar regime label for this period.
    pub regime_label: String,
}

/// 7-KPI summary over daily OHLCV bars matching the Daily OHLC Analysis strip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DailyOhlcSummary {
    /// Latest bar close in range.
    pub latest_close: Decimal,
    /// Latest bar date.
    pub latest_date: Option<NaiveDate>,
    /// Highest high across the range.
    pub period_high: Decimal,
    /// Lowest low across the range.
    pub period_low: Decimal,
    /// Total daily trading bars in range.
    pub total_trading_days: usize,
    /// Mean close-to-close daily return (%).
    pub avg_daily_return_pct: Decimal,
    /// Sample standard deviation of daily returns (%).
    pub daily_return_std_pct: Decimal,
    /// Average daily volume across the range.
    pub avg_volume: Decimal,
}

/// One daily OHLCV row formatted for the interactive Daily OHLC table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyOhlcRow {
    /// Session date.
    pub date: NaiveDate,
    /// Open price.
    pub open: Decimal,
    /// High price.
    pub high: Decimal,
    /// Low price.
    pub low: Decimal,
    /// Close price.
    pub close: Decimal,
    /// Traded volume.
    pub volume: Decimal,
    /// Cash dividend on date.
    pub dividend: Decimal,
    /// Stock split factor on date.
    pub split_factor: Decimal,
    /// Daily close-to-close percentage change.
    pub daily_return_pct: Decimal,
}

/// Computes sample mean and sample standard deviation (`n - 1` denominator when `n >= 2`).
pub(crate) fn mean_and_sample_std(values: &[Decimal]) -> (Decimal, Decimal) {
    let n = values.len();
    if n == 0 {
        return (Decimal::ZERO, Decimal::ZERO);
    }
    let n_dec = Decimal::from(n as i64);
    let sum: Decimal = values.iter().copied().sum();
    let mean = (sum / n_dec).round_dp(8);

    if n < 2 {
        return (mean.round_dp(4), Decimal::ZERO);
    }

    let sq_sum: Decimal = values
        .iter()
        .map(|v| {
            let d = (*v - mean).round_dp(6);
            (d * d).round_dp(10)
        })
        .sum();
    let var = (sq_sum / Decimal::from((n - 1) as i64)).round_dp(10);
    let std_dev = decimal_sqrt(var).round_dp(4);
    (mean.round_dp(4), std_dev)
}

/// Builds the period series with SMA overlay and 3×3 [`ReturnKpis`].
pub fn compute_return_series_and_kpis(
    periods: &[SlicedPeriod],
    ma_window: usize,
) -> (Vec<PeriodPoint>, ReturnKpis) {
    if periods.is_empty() {
        return (Vec::new(), ReturnKpis::default());
    }

    let window = ma_window.max(2);
    let returns: Vec<Decimal> = periods.iter().map(|p| p.return_pct).collect();
    let (mean_pct, std_dev_pct) = mean_and_sample_std(&returns);

    let mut max_gain = returns[0];
    let mut max_loss = returns[0];
    let mut pos_count = 0usize;
    let mut neg_count = 0usize;
    let mut neu_count = 0usize;
    let mut pos_sum = Decimal::ZERO;
    let mut neg_abs_sum = Decimal::ZERO;
    let mut downside_sq_sum = Decimal::ZERO;

    let hundred = Decimal::from(100);
    let n_dec = Decimal::from(returns.len() as i64);

    for &r in &returns {
        if r > max_gain {
            max_gain = r;
        }
        if r < max_loss {
            max_loss = r;
        }
        if r > Decimal::ZERO {
            pos_count += 1;
            pos_sum += r;
        } else if r < Decimal::ZERO {
            neg_count += 1;
            neg_abs_sum += r.abs();
            let r_round = r.round_dp(6);
            downside_sq_sum += (r_round * r_round).round_dp(10);
        } else {
            neu_count += 1;
        }
    }

    let positive_pct = ((Decimal::from(pos_count as i64) / n_dec) * hundred).round_dp(2);
    let negative_pct = ((Decimal::from(neg_count as i64) / n_dec) * hundred).round_dp(2);

    let sharpe_ratio = if std_dev_pct.is_zero() {
        Decimal::ZERO
    } else {
        (mean_pct / std_dev_pct).round_dp(4)
    };

    let downside_dev = if neg_count == 0 {
        Decimal::ZERO
    } else {
        let downside_var = (downside_sq_sum / n_dec).round_dp(10);
        decimal_sqrt(downside_var).round_dp(6)
    };
    let sortino_ratio = if downside_dev.is_zero() {
        Decimal::ZERO
    } else {
        (mean_pct / downside_dev).round_dp(4)
    };

    let profit_factor = if neg_abs_sum.is_zero() {
        if pos_sum > Decimal::ZERO {
            Decimal::from(99)
        } else {
            Decimal::ZERO
        }
    } else {
        (pos_sum / neg_abs_sum).round_dp(4)
    };

    // Rolling SMA
    let mut rolling_sum = Decimal::ZERO;
    let mut points = Vec::with_capacity(periods.len());
    for (idx, p) in periods.iter().enumerate() {
        rolling_sum += p.return_pct;
        if idx >= window {
            rolling_sum -= periods[idx - window].return_pct;
        }
        let ma_pct = if idx + 1 >= window {
            Some((rolling_sum / Decimal::from(window as i64)).round_dp(4))
        } else {
            None
        };
        points.push(PeriodPoint {
            index: p.index,
            start_date: p.start_date,
            end_date: p.end_date,
            entry_price: p.entry_price,
            exit_price: p.exit_price,
            high_price: p.high_price,
            low_price: p.low_price,
            return_pct: p.return_pct,
            ma_pct,
            regime_label: p.regime_label.clone(),
        });
    }

    let kpis = ReturnKpis {
        avg_return_pct: mean_pct.round_dp(2),
        max_gain_pct: max_gain.round_dp(2),
        max_loss_pct: max_loss.round_dp(2),
        std_dev_pct: std_dev_pct.round_dp(2),
        positive_count: pos_count,
        positive_pct,
        negative_count: neg_count,
        negative_pct,
        neutral_count: neu_count,
        sharpe_ratio: sharpe_ratio.round_dp(2),
        sortino_ratio: sortino_ratio.round_dp(2),
        profit_factor: profit_factor.round_dp(2),
    };

    (points, kpis)
}

/// Computes the 7-KPI daily OHLC summary and daily rows with daily returns.
pub fn compute_daily_ohlc_summary(bars: &[OhlcvBar]) -> (DailyOhlcSummary, Vec<DailyOhlcRow>) {
    if bars.is_empty() {
        return (DailyOhlcSummary::default(), Vec::new());
    }

    let mut sorted = bars.to_vec();
    sorted.sort_by_key(|b| b.date);

    let hundred = Decimal::from(100);
    let mut period_high = sorted[0].high;
    let mut period_low = sorted[0].low;
    let mut vol_sum = Decimal::ZERO;
    let mut daily_rets = Vec::with_capacity(sorted.len());
    let mut rows = Vec::with_capacity(sorted.len());

    for (idx, b) in sorted.iter().enumerate() {
        if b.high > period_high {
            period_high = b.high;
        }
        if b.low < period_low {
            period_low = b.low;
        }
        vol_sum += b.volume;
        let ret = if idx == 0 || sorted[idx - 1].close.is_zero() {
            if b.open.is_zero() {
                Decimal::ZERO
            } else {
                (((b.close - b.open) / b.open) * hundred).round_dp(2)
            }
        } else {
            let prev_c = sorted[idx - 1].close;
            (((b.close - prev_c) / prev_c) * hundred).round_dp(2)
        };
        if idx > 0 {
            daily_rets.push(ret);
        }
        rows.push(DailyOhlcRow {
            date: b.date,
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
            dividend: b.dividend,
            split_factor: b.split_factor,
            daily_return_pct: ret,
        });
    }

    let (avg_daily, std_daily) = mean_and_sample_std(&daily_rets);
    let avg_volume = (vol_sum / Decimal::from(sorted.len() as i64)).round_dp(0);
    let latest = sorted.last().unwrap();

    let summary = DailyOhlcSummary {
        latest_close: latest.close,
        latest_date: Some(latest.date),
        period_high,
        period_low,
        total_trading_days: sorted.len(),
        avg_daily_return_pct: avg_daily.round_dp(2),
        daily_return_std_pct: std_daily.round_dp(2),
        avg_volume,
    };

    (summary, rows)
}
