//! Shared domain types and composable strategy primitive traits (`Decimal`-only, `wasm32`-ready).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::super::intraday_orb::IntradayBar;
use super::super::options_events::LiveOptionStrikeQuote;
use super::super::schedule::{OhlcvBar, StudyAssetClass};
use super::super::spreads::SpreadCostConfig;
use crate::calendar::MarketCalendar;

/// Unified bar representation supporting daily/weekly/monthly OHLCV and intraday minute bars.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudyBar {
    /// Local exchange trading date.
    pub date: NaiveDate,
    /// Minutes since midnight local exchange time (`None` for daily/higher-TF bars).
    pub minute_of_day: Option<u16>,
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
    /// Cash dividend on ex-date.
    pub dividend: Decimal,
    /// Split ratio (`<= 1` means no split).
    pub split_factor: Decimal,
}

impl From<&OhlcvBar> for StudyBar {
    fn from(b: &OhlcvBar) -> Self {
        Self {
            date: b.date,
            minute_of_day: None,
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
            dividend: b.dividend,
            split_factor: b.split_factor,
        }
    }
}

impl From<&IntradayBar> for StudyBar {
    fn from(b: &IntradayBar) -> Self {
        Self {
            date: b.date,
            minute_of_day: Some(b.minute_of_day),
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
            dividend: Decimal::ZERO,
            split_factor: Decimal::ZERO,
        }
    }
}

/// Multi-asset session grouping preset for [`super::SessionClock`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionPreset {
    /// NSE equity / cash / regular F&O intraday sessions (`09:15–15:30 IST`).
    NseEquityFno,
    /// NSE Sep 3, 2025 expiry cutover-aware intraday sessions.
    #[default]
    NseSep2025Expiry,
    /// MCX commodity extended intraday sessions (`09:00–23:30 IST`).
    McxCommodity,
    /// Continuous FX sessions (`00:00–23:59` local/UTC).
    FxSession,
    /// Daily bars grouped into calendar-month (`YYYY-MM`) regimes.
    DailyMonthly,
    /// Weekly bars grouped into calendar-quarter (`YYYY-Q#`) regimes.
    WeeklyQuarter,
    /// Monthly bars grouped into calendar-year (`YYYY`) regimes.
    MonthlyYear,
}

/// One contiguous trading session slice produced by [`super::SessionClock`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSlice {
    /// Canonical session identifier (`YYYY-MM-DD`, `YYYY-MM`, `YYYY-Q#`, etc.).
    pub session_key: String,
    /// Inclusive first bar index in input slice.
    pub start_index: usize,
    /// Inclusive last bar index in input slice.
    pub end_index: usize,
    /// First bar open price in this session.
    pub session_open: Decimal,
    /// First bar close price in this session.
    pub anchor_close: Decimal,
    /// Exchange/expiry regime classification tag.
    pub regime_tag: String,
}

/// Detected corporate split / synthetic adjustment action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedSplit {
    /// Index of the ex-split bar.
    pub bar_index: usize,
    /// Ex-split calendar date.
    pub date: NaiveDate,
    /// Detected split multiplier (`> 1`, e.g. `5` for a 5-for-1 split).
    pub factor: Decimal,
    /// `"explicit_split_factor"` or `"synthetic_gap_heuristic"`.
    pub source: String,
}

/// Output from [`super::SplitArtifactGuard`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitCleanReport {
    /// Backward-adjusted continuous bar series.
    pub adjusted_bars: Vec<StudyBar>,
    /// All splits detected and applied chronologically.
    pub detected_splits: Vec<DetectedSplit>,
}

/// Indicator specification evaluated by [`IndicatorPrimitive`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndicatorSpec {
    /// Average True Range (SMA over `period` true ranges).
    Atr {
        /// Lookback period (clamped `>= 1`).
        period: usize,
    },
    /// Exponential Moving Average of close.
    Ema {
        /// Lookback period (clamped `>= 1`).
        period: usize,
    },
    /// Simple Moving Average of close.
    Sma {
        /// Lookback period (clamped `>= 1`).
        period: usize,
    },
    /// Wilder/simple Relative Strength Index (`0..=100`).
    Rsi {
        /// Lookback period (clamped `>= 2`).
        period: usize,
    },
    /// Volume-Weighted Average Price (cumulative/session-anchored).
    Vwap,
    /// Current bar volume divided by trailing `period`-bar average volume.
    VolumeRatio {
        /// Lookback period (clamped `>= 1`).
        period: usize,
    },
}

/// Precomputed technical indicator arrays aligned 1:1 with the bar series.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IndicatorSet {
    /// Primary ATR series aligned by bar index.
    pub atr: Vec<Decimal>,
    /// Named EMA series keyed by period.
    pub ema: BTreeMap<usize, Vec<Decimal>>,
    /// Named SMA series keyed by period.
    pub sma: BTreeMap<usize, Vec<Decimal>>,
    /// RSI (`0..=100`) series if requested.
    pub rsi: Vec<Decimal>,
    /// VWAP series if requested.
    pub vwap: Vec<Decimal>,
    /// Volume ratio series if requested.
    pub volume_ratio: Vec<Decimal>,
}

/// Zone behavioral polarity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZonePolarity {
    /// Demand / support zone.
    Support,
    /// Supply / resistance zone.
    Resistance,
}

/// Generic extracted price band (EE+ `S1/S2/R1/R2`, ORB opening range, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedZoneBand {
    /// Unique identifier (`"{session_key}:{label}"`).
    pub id: String,
    /// Band label (`"S1"`, `"S2"`, `"R1"`, `"R2"`, `"OR_HIGH"`, `"OR_LOW"`).
    pub label: String,
    /// Support vs resistance polarity after flip logic.
    pub polarity: ZonePolarity,
    /// Center price of the band.
    pub center: Decimal,
    /// Lower edge price.
    pub low: Decimal,
    /// Upper edge price.
    pub high: Decimal,
    /// Recommended outer-buffer stop-loss price outside the band.
    pub outer_sl: Decimal,
    /// Session bar start index.
    pub start_index: usize,
    /// Session bar end index.
    pub end_index: usize,
}

/// Reference level line emitted by a [`ZoneExtractor`] (e.g. Equilibrium, ORB Mid).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedLevelLine {
    /// Session key.
    pub session_key: String,
    /// Line label.
    pub label: String,
    /// Line price level.
    pub price: Decimal,
    /// Session bar start index.
    pub start_index: usize,
    /// Session bar end index.
    pub end_index: usize,
}

/// Per-session extracted bands + equilibrium reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionZoneMap {
    /// Session slice metadata.
    pub slice: SessionSlice,
    /// Session equilibrium / midpoint anchor price.
    pub equilibrium: Decimal,
    /// ATR evaluated at session open.
    pub atr_at_open: Decimal,
    /// Outer stop-loss buffer amount in price points.
    pub sl_buffer: Decimal,
    /// Zone bands active in this session.
    pub bands: Vec<ExtractedZoneBand>,
}

/// Complete output of a [`ZoneExtractor`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ZoneExtractionOutput {
    /// Structured per-session maps.
    pub sessions: Vec<SessionZoneMap>,
    /// Flattened bands across all sessions.
    pub bands: Vec<ExtractedZoneBand>,
    /// Reference level lines (e.g. equilibrium).
    pub levels: Vec<ExtractedLevelLine>,
}

/// Directional bias of a detected trade setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalDirection {
    /// Bullish / long setup.
    Long,
    /// Bearish / short setup.
    Short,
}

/// Standardized entry setup emitted by an [`EntrySignalPrimitive`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrySignal {
    /// Entry candle index in bars slice.
    pub bar_index: usize,
    /// Directional bias.
    pub direction: SignalDirection,
    /// Entry execution price (usually bar close).
    pub entry_price: Decimal,
    /// Associated zone band label.
    pub band_label: String,
    /// Band center price.
    pub band_center: Decimal,
    /// Band low edge price.
    pub band_low: Decimal,
    /// Band high edge price.
    pub band_high: Decimal,
    /// Effective ATR at setup bar.
    pub atr: Decimal,
    /// True if signal is also an outer liquidity-sweep reclaim.
    pub is_sweep_reclaim: bool,
    /// Human-readable rationale hint text.
    pub hint: String,
}

/// Stop-loss and trade exit management mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopPolicyMode {
    /// Tight stop placed at band midpoint.
    TightCenterLine,
    /// Stop placed right at band outer edge (`+0.00 ATR`).
    BandOuterEdge,
    /// Stop placed outside band outer edge by `buffer_atr * ATR`.
    OuterAtrBuffer {
        /// Additional ATR buffer beyond band outer edge (`>= 0`).
        buffer_atr: Decimal,
    },
    /// Empirical penetration percentile shield (clamped `0.10..=1.50 ATR`).
    PercentileShield {
        /// Shield width multiplier derived from penetration percentile.
        buffer_atr: Decimal,
    },
    /// Fixed bar horizon exit without intra-horizon tight band tightening.
    FixedHorizon,
    /// Take partial profit at `partial_r`, shift SL to breakeven, trail runner to `runner_r`.
    PartialAndTrail {
        /// Partial target multiple (e.g. `2` or `3` R).
        partial_r: Decimal,
        /// Runner target multiple (e.g. `5` R).
        runner_r: Decimal,
    },
    /// Exit after `bars` elapsed if neither SL nor target hit.
    TimeStop {
        /// Maximum holding bars.
        bars: usize,
    },
}

/// Input bundle passed to [`StopExitPolicy::simulate_trades`].
#[derive(Clone, Debug)]
pub struct StopSimInput<'a> {
    /// Chronological cleaned study bars.
    pub bars: &'a [StudyBar],
    /// Qualifying entry signals to simulate.
    pub signals: &'a [EntrySignal],
    /// Evaluation holding window in bars (`>= 1`).
    pub horizon_bars: usize,
    /// Target reward multiple in `R` units (`>= 1`).
    pub target_r: Decimal,
    /// Instrument symbol for statutory friction lookup.
    pub symbol: &'a str,
    /// Instrument asset class.
    pub asset_class: StudyAssetClass,
    /// Spread/roll/statutory cost assumptions.
    pub spread_cost: &'a SpreadCostConfig,
}

/// Individual simulated trade outcome under a [`StopExitPolicy`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatedTrade {
    /// Entry bar index.
    pub entry_bar_index: usize,
    /// Exit bar index.
    pub exit_bar_index: usize,
    /// Trade direction.
    pub direction: SignalDirection,
    /// Entry price.
    pub entry_price: Decimal,
    /// Stop-loss price.
    pub stop_price: Decimal,
    /// Primary target price (`target_r` multiple of `1R`).
    pub target_price: Decimal,
    /// Exit price realized.
    pub exit_price: Decimal,
    /// Risk unit (`|entry - stop|` floored to `0.1 ATR`).
    pub one_r: Decimal,
    /// True if stopped out before target/horizon.
    pub stopped_out: bool,
    /// True if stopped out while a deep shield (`+0.50 ATR`) survived and closed profitably.
    pub false_sweep_trap: bool,
    /// True if price reached `2R` before exiting.
    pub hit_2r: bool,
    /// True if primary target R was reached.
    pub hit_target: bool,
    /// Gross realized R multiple.
    pub gross_r: Decimal,
    /// Net realized R multiple after configured spread/basis/statutory friction.
    pub net_r: Decimal,
}

/// Aggregated quantitative performance KPIs for a strategy evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StrategyKpis {
    /// Total evaluated trades.
    pub trades: usize,
    /// Count of trades stopped out.
    pub stopped_out_count: usize,
    /// Percentage of trades stopped out.
    pub stop_out_rate_pct: Decimal,
    /// Count of stop-outs where deeper `+0.50 ATR` buffer survived & won.
    pub false_sweep_stop_count: usize,
    /// False sweep trap rate (%).
    pub false_sweep_stop_rate_pct: Decimal,
    /// Winning trade count (`gross_r > 0`).
    pub win_count: usize,
    /// Win rate (%).
    pub win_rate_pct: Decimal,
    /// Rate (%) hitting at least `2R` MFE before exit.
    pub target_2r_hit_rate_pct: Decimal,
    /// Average gross realized R per trade.
    pub avg_realized_r: Decimal,
    /// Average net realized R per trade after spread + statutory costs.
    pub net_avg_realized_r: Decimal,
    /// Gross profit factor (`sum(win R) / sum(|loss R|)`).
    pub profit_factor: Decimal,
    /// Net profit factor after friction.
    pub net_profit_factor: Decimal,
    /// Per-trade Sharpe ratio computed in R-space.
    pub sharpe_ratio: Decimal,
    /// Maximum peak-to-trough equity drawdown measured in R units.
    pub max_drawdown_r: Decimal,
}

/// Empirical excursion percentiles beyond zone outer edges (expressed in ATR units).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PenetrationDistribution {
    /// Number of setups measured.
    pub sample_count: usize,
    /// 50th percentile penetration beyond outer edge (ATR).
    pub p50_atr: Decimal,
    /// 75th percentile penetration beyond outer edge (ATR).
    pub p75_atr: Decimal,
    /// 90th percentile penetration beyond outer edge (ATR).
    pub p90_atr: Decimal,
    /// 95th percentile penetration beyond outer edge (ATR).
    pub p95_atr: Decimal,
    /// Maximum observed penetration beyond outer edge (ATR).
    pub max_atr: Decimal,
    /// Percentage of setups breaching outer edge that still closed profitable by horizon.
    pub pct_recovering_after_outer_touch: Decimal,
}

/// One stop-loss policy row inside a 4-model comparison benchmark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopComparisonRow {
    /// Canonical identifier (`"tight_center"`, `"outer_edge_0"`, `"outer_025_atr"`, `"outer_050_atr"`).
    pub policy_id: String,
    /// Human-readable label.
    pub label: String,
    /// Explanation of stop placement rule.
    pub description: String,
    /// ATR buffer relative to band outer edge.
    pub buffer_atr: Decimal,
    /// Complete KPIs under this stop policy.
    pub kpis: StrategyKpis,
}

/// Target derivative expiry / holding horizon for hedge synthesis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryHorizon {
    /// Intraday `0–1 DTE`.
    Day,
    /// Weekly `2–5 DTE`.
    #[default]
    Weekly,
    /// Monthly `5–30 DTE`.
    Monthly,
    /// MCX commodity roll cycle.
    CommodityCycle,
}

/// Recommended F&O option / futures hedge family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HedgeStructureKind {
    /// Naked underlying / spot direction with outer-buffer stop shield.
    #[default]
    Unhedged,
    /// Bullish defined-risk vertical call spread.
    BullCallSpread,
    /// Bearish defined-risk vertical put spread.
    BearPutSpread,
    /// Bullish credit spread anchored below `S1`/`S2` outer SL.
    BullPutCreditSpread,
    /// Bearish credit spread anchored above `R1`/`R2` outer SL.
    BearCallCreditSpread,
    /// Range-bound 4-leg Iron Condor anchored outside `S1`/`R1` buffers.
    ZoneIronCondor,
    /// Long/short future/spot paired with outer wing protection & OTM cap.
    ProtectiveCollar,
    /// Near/next expiry calendar roll.
    CalendarRoll,
}

/// Single leg inside a synthesized option/futures hedge recommendation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HedgeLeg {
    /// `"BUY"` or `"SELL"`.
    pub side: String,
    /// `"FUT"`, `"CE"`, or `"PE"`.
    pub option_type: String,
    /// Strike price (or entry spot for futures/underlying leg).
    pub strike: Decimal,
    /// Expiry date when sourced from live chain.
    pub expiry: Option<NaiveDate>,
    /// Lot-rounded contract quantity.
    pub lot_qty: Decimal,
    /// Estimated/quoted leg price per unit.
    pub leg_price: Decimal,
    /// Source of leg price (`"live_bid_ask"`, `"live_ltp"`, or `"theoretical_atr"`).
    pub quote_source: String,
}

/// Inputs required by [`HedgeOverlayPolicy`] to synthesize an expiry-aware hedge.
#[derive(Clone, Debug)]
pub struct HedgeSynthesisInput<'a> {
    /// Instrument symbol.
    pub symbol: String,
    /// Asset class context.
    pub asset_class: StudyAssetClass,
    /// Current underlying spot price.
    pub spot: Decimal,
    /// Current ATR.
    pub atr: Decimal,
    /// Target F&O / commodity horizon.
    pub horizon: ExpiryHorizon,
    /// Active directional bias (`None` implies neutral/range regime).
    pub direction: Option<SignalDirection>,
    /// Latest extracted zone bands (`None` uses ATR-implied levels).
    pub zones: Option<&'a ZoneExtractionOutput>,
    /// Optional penetration statistics to anchor wings past `P90`/`P95`.
    pub penetration: Option<&'a PenetrationDistribution>,
    /// True when implied vol exceeds realized vol (`IV > RV`).
    pub iv_gt_rv: bool,
    /// Observed false-sweep trap rate (%).
    pub trap_rate_pct: Decimal,
    /// Live option chain quotes (`&[]` triggers honest theoretical fallback).
    pub live_chain: &'a [LiveOptionStrikeQuote],
    /// Contract lot size (`>= 1`).
    pub lot_size: Decimal,
    /// Strike/price tick size (`>= 0.01`).
    pub tick_size: Decimal,
    /// Portfolio / strategy allocated capital (₹) for % equity risk sizing.
    pub capital: Decimal,
    /// Friction config (spread + statutory costs).
    pub spread_cost: &'a SpreadCostConfig,
}

/// Complete synthesized hedge recommendation emitted by [`HedgeOverlayPolicy`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HedgeRecommendation {
    /// Selected hedge structure family.
    pub structure: HedgeStructureKind,
    /// Target expiry horizon.
    pub horizon: ExpiryHorizon,
    /// `"live_quotes"` if live option quotes matched, else `"theoretical_model"`.
    pub chain_status: String,
    /// Individual legs of the structure.
    pub legs: Vec<HedgeLeg>,
    /// Estimated net premium / cost per lot (`> 0` credit, `< 0` debit).
    pub net_credit_or_debit: Decimal,
    /// Worst-case bounded loss (₹ across 1 lot / sized qty) under structure or `P95` stress.
    pub max_loss_inr: Decimal,
    /// Max loss as percentage of allocated `capital`.
    pub max_loss_pct_equity: Decimal,
    /// `P95` adverse gap stress loss estimate (₹).
    pub p95_stress_loss_inr: Decimal,
    /// Estimated round-trip spread + statutory cost (₹).
    pub estimated_friction_inr: Decimal,
    /// Lower and upper breakeven price levels if bounded.
    pub breakevens: Vec<Decimal>,
    /// Clear rationale explaining why this structure matches horizon & zone geometry.
    pub rationale: String,
}

/// Request specification for [`super::StrategyLoopKernel::evaluate`] and [`super::StrategyLoopKernel::compare_stop_policies`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopEvalRequest {
    /// Identifier of strategy (`"elephant_edge_v3"`, `"orb_15"`, etc.).
    pub strategy_id: String,
    /// Instrument symbol.
    pub symbol: String,
    /// Asset class.
    pub asset_class: StudyAssetClass,
    /// Session clock slicing preset.
    pub session_preset: SessionPreset,
    /// Chronological bars (oldest first).
    pub bars: Vec<StudyBar>,
    /// Evaluation horizon in bars after signal entry (`>= 1`).
    pub horizon_bars: usize,
    /// Target reward multiple `R` (`>= 1`).
    pub target_r: Decimal,
    /// ATR lookback period (`>= 2`).
    pub atr_period: usize,
    /// Outer stop-loss buffer multiplier in ATR (`>= 0`).
    pub sl_buffer_atr: Decimal,
    /// Target derivative horizon for hedge synthesis.
    pub expiry_horizon: ExpiryHorizon,
    /// Spread / roll / statutory friction assumptions.
    pub spread_cost: SpreadCostConfig,
    /// Optional live option chain snapshots.
    pub live_chain: Vec<LiveOptionStrikeQuote>,
}

/// Output produced by [`super::StrategyLoopKernel::evaluate`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopEvalReport {
    /// Strategy identifier.
    pub strategy_id: String,
    /// Symbol evaluated.
    pub symbol: String,
    /// Total bars after split hygiene cleaning.
    pub total_bars: usize,
    /// Number of splits corrected by `SplitArtifactGuard`.
    pub splits_adjusted: usize,
    /// Extracted zones across sessions.
    pub zones: ZoneExtractionOutput,
    /// All entry setups detected.
    pub signals: Vec<EntrySignal>,
    /// Simulated trades under the selected stop policy.
    pub trades: Vec<SimulatedTrade>,
    /// Empirical excursion beyond band outer edges.
    pub penetration: PenetrationDistribution,
    /// Aggregated KPI summary.
    pub kpis: StrategyKpis,
    /// Recommended hedge for the latest market state.
    pub hedge: HedgeRecommendation,
}

/// Side-by-side 4-policy stop-hunt benchmark comparison report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopComparisonReport {
    /// Strategy identifier.
    pub strategy_id: String,
    /// Total bars evaluated.
    pub total_bars: usize,
    /// Total qualifying setups detected.
    pub total_setups: usize,
    /// Count of bullish setups.
    pub bullish_setups: usize,
    /// Count of bearish setups.
    pub bearish_setups: usize,
    /// Empirical penetration distribution beyond outer edges.
    pub penetration: PenetrationDistribution,
    /// Four compared stop models (`tight_center`, `outer_edge_0`, `outer_025_atr`, `outer_050_atr`).
    pub models: Vec<StopComparisonRow>,
    /// Policy ID achieving highest net expectancy.
    pub recommended_policy_id: String,
    /// Concise verdict summary highlighting trap reduction.
    pub verdict_summary: String,
}

/// Bounded parameter search space for [`super::ParameterSweepRunner`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyParameterBounds {
    /// Candidate ATR stop buffer values (e.g. `[0.00, 0.25, 0.50]`).
    pub sl_buffer_atr_grid: Vec<Decimal>,
    /// Candidate holding horizon bar counts (e.g. `[3, 5, 10]`).
    pub horizon_bars_grid: Vec<usize>,
    /// Candidate reward target R multiples (e.g. `[2, 3, 5]`).
    pub target_r_grid: Vec<Decimal>,
    /// Hard upper cap on evaluated combinations (enforced `<= 64`).
    pub max_candidates: usize,
}

/// Sweep execution request for [`super::ParameterSweepRunner::run`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterSweepRequest {
    /// Base request template (overridden per grid point).
    pub base: LoopEvalRequest,
    /// Parameter grid bounds.
    pub bounds: StrategyParameterBounds,
}

/// One candidate parameter configuration scored on IS, OOS, Full, WFE & DSR.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepCandidateResult {
    /// Zero-based candidate ordinal.
    pub candidate_index: usize,
    /// Tested ATR buffer.
    pub sl_buffer_atr: Decimal,
    /// Tested holding horizon bars.
    pub horizon_bars: usize,
    /// Tested target R.
    pub target_r: Decimal,
    /// Full-window KPIs.
    pub full_kpis: StrategyKpis,
    /// In-sample (70%) KPIs.
    pub is_kpis: StrategyKpis,
    /// Out-of-sample (30%) KPIs.
    pub oos_kpis: StrategyKpis,
    /// Walk-Forward Efficiency `(OOS avg R / IS avg R) * 100`.
    pub walk_forward_efficiency_pct: Decimal,
    /// Deflated Sharpe Ratio proxy adjusting for `trial_count` multiple testing.
    pub deflated_sharpe_ratio: Decimal,
    /// Recommended hedge for the winning / evaluated parameter point.
    pub hedge: HedgeRecommendation,
}

/// Complete parameter sweep + overfitting guard report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterSweepReport {
    /// Strategy identifier.
    pub strategy_id: String,
    /// Number of parameter combinations actually tested (`<= 64`).
    pub trial_count: usize,
    /// Ranked candidates (best OOS expectancy first).
    pub candidates: Vec<SweepCandidateResult>,
    /// Winning candidate configuration.
    pub winner: SweepCandidateResult,
}

/// Composable primitive 2: extracts zones and reference levels from bars & session slices.
pub trait ZoneExtractor: Send + Sync {
    /// Unique identifier for this zone extractor.
    fn id(&self) -> &'static str;

    /// Extracts per-session bands and reference level lines cleanly.
    fn extract(
        &self,
        bars: &[StudyBar],
        sessions: &[SessionSlice],
        indicators: &IndicatorSet,
    ) -> ZoneExtractionOutput;
}

/// Composable primitive 3: computes named technical indicator series from bars.
pub trait IndicatorPrimitive: Send + Sync {
    /// Evaluates requested [`IndicatorSpec`]s across `bars`.
    fn evaluate(&self, bars: &[StudyBar], specs: &[IndicatorSpec]) -> IndicatorSet;
}

/// Composable primitive 4: detects standardized entry signals against extracted zones.
pub trait EntrySignalPrimitive: Send + Sync {
    /// Unique identifier for this signal filter.
    fn id(&self) -> &'static str;

    /// Scans `bars`, `zones`, and `indicators` to emit qualifying [`EntrySignal`] setups.
    fn detect_signals(
        &self,
        bars: &[StudyBar],
        zones: &ZoneExtractionOutput,
        indicators: &IndicatorSet,
    ) -> Vec<EntrySignal>;
}

/// Composable primitive 5: places stop-losses and simulates trade exit paths.
pub trait StopExitPolicy: Send + Sync {
    /// Canonical identifier of this stop/exit policy.
    fn policy_id(&self) -> String;

    /// Active policy mode.
    fn mode(&self) -> StopPolicyMode;

    /// Computes initial stop-loss price for `signal`.
    fn stop_price(&self, signal: &EntrySignal) -> Decimal;

    /// Simulates all signals in `input` forward over `bars` cleanly.
    fn simulate_trades(&self, input: &StopSimInput<'_>) -> (Vec<SimulatedTrade>, StrategyKpis);
}

/// Composable primitive 6: synthesizes horizon-aware F&O / commodity hedges.
pub trait HedgeOverlayPolicy: Send + Sync {
    /// Synthesizes legs, sizing, stress risk, and rationale for `input`.
    fn synthesize(&self, input: &HedgeSynthesisInput<'_>) -> HedgeRecommendation;
}

/// Shared calendar utility wrapper signature helper for internal callers.
pub(crate) fn _ensure_calendar_trait(_cal: &dyn MarketCalendar) {}
