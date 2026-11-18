//! Multi-asset statistical & event study quantitative engine (`Decimal`-only, `wasm32`-ready).
//!
//! Covers:
//! - Regime-aware period slicing (`schedule`) across Indices, F&O Stocks, ETFs, Equities, Forex & Commodities
//! - F&O spread, futures roll-basis & statutory cost adjustments (`spreads`)
//! - Period return KPIs & rolling moving average (`returns`)
//! - Directional streak runs & 6-state Markov transition matrix (`streaks_markov`)
//! - Empirical percentiles, 16-bucket & 10-bps distributions, Probability-Based Levels,
//!   Green-vs-Red comparisons, conditional forward returns & survivability outliers (`distributions`)
//! - Intraday `5m` movement, overnight gaps, 5-day rolling move, 0-100% cycle progress, and
//!   15m ORB Flip backtester (`intraday_orb`)
//! - Cross-asset N×N Pearson/Spearman correlation, rolling beta & pair spread (`correlation`)
//! - Multi-leg F&O options overlay & strike lab, Markov+vol analog forecasting, and
//!   `CAR[-20d, +20d]` event study (`options_events`)

pub mod correlation;
pub mod distributions;
pub mod intraday_orb;
pub mod options_events;
pub mod returns;
pub mod schedule;
pub mod spreads;
pub mod streaks_markov;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::calendar::MarketCalendar;

pub use correlation::{CorrelationAnalysis, RollingPairPoint, analyze_cross_asset_correlation};
pub use distributions::{
    DistributionBundle, ForwardConditionEvent, ForwardConditionalGroup, GreenVsRedComparison,
    MagnitudePdfBucket, OutlierAnalysis, OutlierKpis, OutlierPeriodPoint, OutlierStatus,
    PolarityStats, ProbabilityLevelRow, ReturnBucketRow, ReturnPercentiles,
    analyze_distributions_and_outliers, empirical_quantile,
};
pub use intraday_orb::{
    CycleProgressAnalysis, CycleProgressDecile, GapAnalysis, GapBucketRow, GapEventRow,
    IntradayBar, IntradayMovementAnalysis, IntradaySlotStat, OrbBacktestResult, OrbConfig,
    OrbEquityPoint, OrbTradeRow, RollingFiveDayAnalysis, RollingFiveDayPoint,
    analyze_cycle_progress, analyze_gaps, analyze_intraday_movement, analyze_rolling_5day,
    run_orb15_flip_backtest,
};
pub use options_events::{
    AnalogEpisode, EventStudyReport, EventStudyRow, EventWindowPoint, ForecastHorizonStep,
    ForecastReport, LiveOptionStrikeQuote, OptionLegSpec, OptionOverlayStrategy,
    OptionsOverlayConfig, OptionsOverlayReport, OverlayEquityPoint, StrikeLabRow, StudyEventInput,
    StudyEventKind, VolatilityRegime, evaluate_event_study, evaluate_forecasting,
    evaluate_options_overlay,
};
pub use returns::{
    DailyOhlcRow, DailyOhlcSummary, PeriodPoint, ReturnKpis, compute_daily_ohlc_summary,
    compute_return_series_and_kpis,
};
pub use schedule::{
    OhlcvBar, PeriodScheduleKind, ReturnCalcKind, SlicedPeriod, StudyAssetClass, SubCycleKind,
    slice_periods,
};
pub use spreads::{
    FrictionSummary, RollBasisMode, SpreadCostConfig, apply_spread_and_basis,
    estimate_statutory_roundtrip_bps,
};
pub use streaks_markov::{
    MarkovAnalysis, MarkovCurrentPrediction, MarkovState, MarkovStateProbability,
    MarkovTransitionRow, SideStreakStats, StreakAnalysis, StreakDetail, StreakDirection,
    StreakLengthBucket, StreakSummary, analyze_streaks_and_markov,
};

/// Errors emitted when validating or executing an empirical study.
#[derive(Debug, Error)]
pub enum StudyError {
    /// No bars supplied in the selected date window.
    #[error("no OHLCV bars available for instrument {symbol} in requested date range")]
    EmptyBars {
        /// Requested instrument symbol.
        symbol: String,
    },
    /// Insufficient bars to form at least one complete period under the active schedule.
    #[error("insufficient bars to slice at least one period for {symbol}")]
    NoPeriods {
        /// Requested instrument symbol.
        symbol: String,
    },
}

/// Parameters controlling a Phase 1/2 Returns, Distribution & Outlier study evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentalStudyConfig {
    /// Instrument symbol/id used to detect NSE/BSE regime rules cleanly.
    pub symbol: String,
    /// Asset class category.
    pub asset_class: StudyAssetClass,
    /// Boundary slicing schedule.
    pub schedule: PeriodScheduleKind,
    /// Current period vs carry `+1` period.
    pub sub_cycle: SubCycleKind,
    /// Return formula (`OC`, `H-L`, `CC`).
    pub calc_kind: ReturnCalcKind,
    /// Rolling SMA window over period returns (clamped to `2..=120`).
    pub ma_window: usize,
    /// Outlier survivability boundary coverage percentage (`50..=99`).
    pub outlier_coverage_pct: Decimal,
    /// Absolute return threshold (%) triggering forward conditional analysis.
    pub tail_threshold_pct: Decimal,
    /// Optional live spot price override (`None` falls back to latest bar close).
    pub live_spot_override: Option<Decimal>,
    /// Optional F&O spread, futures roll basis & statutory fee config (Phase 2).
    #[serde(default)]
    pub spread_cost: SpreadCostConfig,
}

/// Complete statistical study bundle computed cleanly in exact `Decimal`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentalStudyReport {
    /// Regime transition note if the requested range spans an exchange cutover.
    pub regime_note: Option<String>,
    /// Optional Gross vs Net comparison when F&O spread/basis/statutory costs are applied.
    pub friction_summary: Option<FrictionSummary>,
    /// 3×3 return summary metrics.
    pub kpis: ReturnKpis,
    /// Sliced periods + rolling MA values.
    pub period_series: Vec<PeriodPoint>,
    /// Directional streaks & duration histogram.
    pub streaks: StreakAnalysis,
    /// 6-state Markov transition matrix & steady-state equilibrium.
    pub markov: MarkovAnalysis,
    /// Percentiles, 16-bucket/10-bps bins, probability levels, green-vs-red, forward & outliers.
    pub distributions: DistributionBundle,
    /// Daily OHLC 7-KPI summary.
    pub daily_ohlc_summary: DailyOhlcSummary,
    /// Daily OHLC rows in ascending date order.
    pub daily_ohlc_rows: Vec<DailyOhlcRow>,
}

/// Deterministic fixed-point Newton–Raphson `Decimal` square root (`x >= 0`).
pub(crate) fn decimal_sqrt(x: Decimal) -> Decimal {
    if x <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    if x == Decimal::ONE {
        return Decimal::ONE;
    }
    let two = Decimal::from(2);
    let mut guess = if x > Decimal::from(4) {
        (x / two).round_dp(12)
    } else {
        Decimal::ONE
    };
    let eps = Decimal::new(1, 11);
    for _ in 0..32 {
        if guess.is_zero() {
            return Decimal::ZERO;
        }
        let next = ((guess + (x / guess).round_dp(12)) / two).round_dp(12);
        if (next - guess).abs() <= eps {
            return next;
        }
        guess = next;
    }
    guess
}

/// Evaluates the complete Phase 1 + 2 `Experimental-Analysis` statistical study on `bars`.
pub fn evaluate_returns_study(
    bars: &[OhlcvBar],
    calendar: &dyn MarketCalendar,
    config: &ExperimentalStudyConfig,
) -> Result<ExperimentalStudyReport, StudyError> {
    if bars.is_empty() {
        return Err(StudyError::EmptyBars {
            symbol: config.symbol.clone(),
        });
    }

    let (mut periods, regime_note) = slice_periods(
        bars,
        &config.symbol,
        config.asset_class,
        config.schedule,
        config.sub_cycle,
        config.calc_kind,
        calendar,
    );

    if periods.is_empty() {
        return Err(StudyError::NoPeriods {
            symbol: config.symbol.clone(),
        });
    }

    let ma_w = config.ma_window.clamp(2, 120);
    let friction_summary = apply_spread_and_basis(
        &mut periods,
        config.asset_class,
        &config.symbol,
        config.calc_kind,
        &config.spread_cost,
        ma_w,
    );

    let (period_series, kpis) = compute_return_series_and_kpis(&periods, ma_w);
    let (streaks, markov) = analyze_streaks_and_markov(&periods);
    let (daily_ohlc_summary, daily_ohlc_rows) = compute_daily_ohlc_summary(bars);

    let ref_price = periods.last().unwrap().entry_price;
    let spot = config
        .live_spot_override
        .unwrap_or(daily_ohlc_summary.latest_close);

    let distributions = analyze_distributions_and_outliers(
        &periods,
        spot,
        ref_price,
        config.outlier_coverage_pct,
        config.tail_threshold_pct,
    );

    Ok(ExperimentalStudyReport {
        regime_note,
        friction_summary,
        kpis,
        period_series,
        streaks,
        markov,
        distributions,
        daily_ohlc_summary,
        daily_ohlc_rows,
    })
}
