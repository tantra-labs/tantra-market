//! Layer B: Generic [`StrategyLoopKernel`] and bounded IS/OOS + DSR [`ParameterSweepRunner`].

use std::cmp::Ordering;

use rust_decimal::Decimal;

use super::super::{StudyError, decimal_sqrt};
use super::hygiene::{SessionClock, SplitArtifactGuard, StandardIndicatorEvaluator};
use super::stops::{
    OuterBufferStopPolicy, compute_penetration_distribution, summarize_trades_kpis,
};
use super::types::{
    EntrySignalPrimitive, HedgeOverlayPolicy, HedgeSynthesisInput, IndicatorPrimitive,
    IndicatorSpec, LoopEvalReport, LoopEvalRequest, ParameterSweepReport, ParameterSweepRequest,
    StopComparisonReport, StopComparisonRow, StopExitPolicy, StopPolicyMode, StopSimInput,
    SweepCandidateResult, ZoneExtractor,
};
use crate::calendar::MarketCalendar;

/// Generic execution and benchmarking kernel connecting any [`ZoneExtractor`],
/// [`EntrySignalPrimitive`], [`StopExitPolicy`], and [`HedgeOverlayPolicy`].
pub struct StrategyLoopKernel;

impl StrategyLoopKernel {
    /// Evaluates one strategy configuration cleanly in pure `Decimal`.
    pub fn evaluate(
        req: &LoopEvalRequest,
        calendar: &dyn MarketCalendar,
        zone_extractor: &dyn ZoneExtractor,
        entry_filter: &dyn EntrySignalPrimitive,
        stop_policy: &dyn StopExitPolicy,
        hedge_policy: &dyn HedgeOverlayPolicy,
    ) -> Result<LoopEvalReport, StudyError> {
        if req.bars.is_empty() {
            return Err(StudyError::EmptyBars {
                symbol: req.symbol.clone(),
            });
        }

        let clean = SplitArtifactGuard::default().clean(&req.bars);
        let bars = &clean.adjusted_bars;
        let clock = SessionClock::new(req.session_preset);
        let sessions = clock.slice_sessions(bars, req.asset_class, calendar);
        if sessions.is_empty() {
            return Err(StudyError::NoPeriods {
                symbol: req.symbol.clone(),
            });
        }

        let indicators = StandardIndicatorEvaluator.evaluate(
            bars,
            &[
                IndicatorSpec::Atr {
                    period: req.atr_period.max(2),
                },
                IndicatorSpec::Ema { period: 20 },
                IndicatorSpec::Rsi { period: 14 },
            ],
        );

        let zones = zone_extractor.extract(bars, &sessions, &indicators);
        let signals = entry_filter.detect_signals(bars, &zones, &indicators);
        let (trades, kpis) = stop_policy.simulate_trades(&StopSimInput {
            bars,
            signals: &signals,
            horizon_bars: req.horizon_bars,
            target_r: req.target_r,
            symbol: &req.symbol,
            asset_class: req.asset_class,
            spread_cost: &req.spread_cost,
        });
        let penetration = compute_penetration_distribution(bars, &signals, req.horizon_bars);

        let spot = bars.last().map(|b| b.close).unwrap_or(Decimal::from(100));
        let atr = indicators
            .atr
            .last()
            .copied()
            .unwrap_or_else(|| spot * Decimal::new(1, 2));
        let last_dir = signals.last().map(|s| s.direction);

        let hedge = hedge_policy.synthesize(&HedgeSynthesisInput {
            symbol: req.symbol.clone(),
            asset_class: req.asset_class,
            spot,
            atr,
            horizon: req.expiry_horizon,
            direction: last_dir,
            zones: Some(&zones),
            penetration: Some(&penetration),
            iv_gt_rv: false,
            trap_rate_pct: kpis.false_sweep_stop_rate_pct,
            live_chain: &req.live_chain,
            lot_size: Decimal::from(50),
            tick_size: Decimal::new(5, 2),
            capital: Decimal::from(500_000),
            spread_cost: &req.spread_cost,
        });

        Ok(LoopEvalReport {
            strategy_id: req.strategy_id.clone(),
            symbol: req.symbol.clone(),
            total_bars: bars.len(),
            splits_adjusted: clean.detected_splits.len(),
            zones,
            signals,
            trades,
            penetration,
            kpis,
            hedge,
        })
    }

    /// Runs the canonical 4-policy stop-hunt benchmark (`tight_center`, `outer_edge_0`,
    /// `outer_025_atr`, `outer_050_atr`) on ANY strategy (`ZoneExtractor` + `EntrySignalPrimitive`).
    pub fn compare_stop_policies(
        req: &LoopEvalRequest,
        calendar: &dyn MarketCalendar,
        zone_extractor: &dyn ZoneExtractor,
        entry_filter: &dyn EntrySignalPrimitive,
    ) -> Result<StopComparisonReport, StudyError> {
        if req.bars.is_empty() {
            return Err(StudyError::EmptyBars {
                symbol: req.symbol.clone(),
            });
        }

        let clean = SplitArtifactGuard::default().clean(&req.bars);
        let bars = &clean.adjusted_bars;
        let clock = SessionClock::new(req.session_preset);
        let sessions = clock.slice_sessions(bars, req.asset_class, calendar);
        if sessions.is_empty() {
            return Err(StudyError::NoPeriods {
                symbol: req.symbol.clone(),
            });
        }

        let indicators = StandardIndicatorEvaluator.evaluate(
            bars,
            &[IndicatorSpec::Atr {
                period: req.atr_period.max(2),
            }],
        );
        let zones = zone_extractor.extract(bars, &sessions, &indicators);
        let signals = entry_filter.detect_signals(bars, &zones, &indicators);
        let penetration = compute_penetration_distribution(bars, &signals, req.horizon_bars);

        let specs = [
            (
                "tight_center",
                "Tight Line SL (Center)",
                "Usual single-line stop inside zone midpoint",
                Decimal::new(-35, 2),
                StopPolicyMode::TightCenterLine,
            ),
            (
                "outer_edge_0",
                "Band Outer Edge (+0.00 ATR)",
                "Exact far edge of band, zero extra sweep cushion",
                Decimal::ZERO,
                StopPolicyMode::BandOuterEdge,
            ),
            (
                "outer_025_atr",
                "EE+ Outer Buffer (+0.25 ATR)",
                "Elephant Edge default: outer band edge + 0.25xATR",
                Decimal::new(25, 2),
                StopPolicyMode::OuterAtrBuffer {
                    buffer_atr: Decimal::new(25, 2),
                },
            ),
            (
                "outer_050_atr",
                "Deep Shield (+0.50 ATR)",
                "Wide institutional buffer outside outer edge + 0.50xATR",
                Decimal::new(50, 2),
                StopPolicyMode::OuterAtrBuffer {
                    buffer_atr: Decimal::new(50, 2),
                },
            ),
        ];

        let mut models = Vec::with_capacity(4);
        for (id, label, desc, buffer_atr, mode) in specs {
            let policy = OuterBufferStopPolicy::new(mode);
            let (_, kpis) = policy.simulate_trades(&StopSimInput {
                bars,
                signals: &signals,
                horizon_bars: req.horizon_bars,
                target_r: req.target_r,
                symbol: &req.symbol,
                asset_class: req.asset_class,
                spread_cost: &req.spread_cost,
            });
            models.push(StopComparisonRow {
                policy_id: id.to_string(),
                label: label.to_string(),
                description: desc.to_string(),
                buffer_atr,
                kpis,
            });
        }

        let best = models
            .iter()
            .max_by(|a, b| a.kpis.net_avg_realized_r.cmp(&b.kpis.net_avg_realized_r))
            .unwrap_or(&models[0]);

        let tight = &models[0];
        let ee_def = &models[2];
        let stop_reduction =
            (tight.kpis.stop_out_rate_pct - ee_def.kpis.stop_out_rate_pct).round_dp(2);

        let bullish = signals
            .iter()
            .filter(|s| s.direction == super::types::SignalDirection::Long)
            .count();
        let bearish = signals.len().saturating_sub(bullish);

        let verdict_summary = format!(
            "Moving from tight center SL ({}% stopped, {}% traps) to EE+ Outer Buffer +0.25xATR ({}% stopped) cuts stop-outs by {} pp. Best expectancy: {} ({:+.3}R/trade, PF {}).",
            tight.kpis.stop_out_rate_pct,
            tight.kpis.false_sweep_stop_rate_pct,
            ee_def.kpis.stop_out_rate_pct,
            stop_reduction,
            best.label,
            best.kpis.net_avg_realized_r,
            best.kpis.net_profit_factor,
        );
        let recommended_policy_id = best.policy_id.clone();

        Ok(StopComparisonReport {
            strategy_id: req.strategy_id.clone(),
            total_bars: bars.len(),
            total_setups: signals.len(),
            bullish_setups: bullish,
            bearish_setups: bearish,
            penetration,
            models,
            recommended_policy_id,
            verdict_summary,
        })
    }
}

/// Bounded multi-parameter grid sweep runner with In-Sample / Out-of-Sample split,
/// Walk-Forward Efficiency (`WFE %`), and pure-`Decimal` Deflated Sharpe Ratio (`DSR`).
pub struct ParameterSweepRunner;

impl ParameterSweepRunner {
    /// Executes bounded grid search (`max_candidates <= 64`) cleanly over IS (70%) / OOS (30%).
    pub fn run(
        req: &ParameterSweepRequest,
        calendar: &dyn MarketCalendar,
        zone_extractor: &dyn ZoneExtractor,
        entry_filter: &dyn EntrySignalPrimitive,
        hedge_policy: &dyn HedgeOverlayPolicy,
    ) -> Result<ParameterSweepReport, StudyError> {
        if req.base.bars.is_empty() {
            return Err(StudyError::EmptyBars {
                symbol: req.base.symbol.clone(),
            });
        }

        let cap = req.bounds.max_candidates.clamp(1, 64);
        let buffers = if req.bounds.sl_buffer_atr_grid.is_empty() {
            vec![req.base.sl_buffer_atr]
        } else {
            req.bounds.sl_buffer_atr_grid.clone()
        };
        let horizons = if req.bounds.horizon_bars_grid.is_empty() {
            vec![req.base.horizon_bars.max(1)]
        } else {
            req.bounds.horizon_bars_grid.clone()
        };
        let targets = if req.bounds.target_r_grid.is_empty() {
            vec![req.base.target_r.max(Decimal::ONE)]
        } else {
            req.bounds.target_r_grid.clone()
        };

        let mut combos = Vec::new();
        'outer: for &b in &buffers {
            for &h in &horizons {
                for &t in &targets {
                    combos.push((b, h.max(1), t.max(Decimal::ONE)));
                    if combos.len() >= cap {
                        break 'outer;
                    }
                }
            }
        }

        let trial_count = combos.len();
        let clean = SplitArtifactGuard::default().clean(&req.base.bars);
        let bars = &clean.adjusted_bars;
        let clock = SessionClock::new(req.base.session_preset);
        let sessions = clock.slice_sessions(bars, req.base.asset_class, calendar);
        if sessions.is_empty() {
            return Err(StudyError::NoPeriods {
                symbol: req.base.symbol.clone(),
            });
        }

        let indicators = StandardIndicatorEvaluator.evaluate(
            bars,
            &[IndicatorSpec::Atr {
                period: req.base.atr_period.max(2),
            }],
        );
        let zones = zone_extractor.extract(bars, &sessions, &indicators);
        let signals = entry_filter.detect_signals(bars, &zones, &indicators);

        let split_bar_idx = ((bars.len() * 7) / 10).max(1);
        let spot = bars.last().map(|b| b.close).unwrap_or(Decimal::from(100));
        let atr = indicators
            .atr
            .last()
            .copied()
            .unwrap_or_else(|| spot * Decimal::new(1, 2));

        let mut candidates = Vec::with_capacity(trial_count);
        for (c_idx, (sl_buf, horizon_bars, target_r)) in combos.into_iter().enumerate() {
            let mode = if sl_buf.is_zero() {
                StopPolicyMode::BandOuterEdge
            } else {
                StopPolicyMode::OuterAtrBuffer { buffer_atr: sl_buf }
            };
            let policy = OuterBufferStopPolicy::new(mode);
            let (full_trades, full_kpis) = policy.simulate_trades(&StopSimInput {
                bars,
                signals: &signals,
                horizon_bars,
                target_r,
                symbol: &req.base.symbol,
                asset_class: req.base.asset_class,
                spread_cost: &req.base.spread_cost,
            });

            let is_trades: Vec<_> = full_trades
                .iter()
                .filter(|t| t.entry_bar_index < split_bar_idx)
                .cloned()
                .collect();
            let oos_trades: Vec<_> = full_trades
                .iter()
                .filter(|t| t.entry_bar_index >= split_bar_idx)
                .cloned()
                .collect();

            let is_kpis = summarize_trades_kpis(&is_trades);
            let oos_kpis = summarize_trades_kpis(&oos_trades);

            let wfe_pct = if is_kpis.net_avg_realized_r > Decimal::ZERO {
                ((oos_kpis.net_avg_realized_r / is_kpis.net_avg_realized_r) * Decimal::from(100))
                    .round_dp(2)
            } else if oos_kpis.net_avg_realized_r > Decimal::ZERO {
                Decimal::from(100)
            } else {
                Decimal::ZERO
            };

            let dsr = compute_deflated_sharpe(full_kpis.sharpe_ratio, trial_count);
            let pen = compute_penetration_distribution(bars, &signals, horizon_bars);
            let hedge = hedge_policy.synthesize(&HedgeSynthesisInput {
                symbol: req.base.symbol.clone(),
                asset_class: req.base.asset_class,
                spot,
                atr,
                horizon: req.base.expiry_horizon,
                direction: signals.last().map(|s| s.direction),
                zones: Some(&zones),
                penetration: Some(&pen),
                iv_gt_rv: false,
                trap_rate_pct: full_kpis.false_sweep_stop_rate_pct,
                live_chain: &req.base.live_chain,
                lot_size: Decimal::from(50),
                tick_size: Decimal::new(5, 2),
                capital: Decimal::from(500_000),
                spread_cost: &req.base.spread_cost,
            });

            candidates.push(SweepCandidateResult {
                candidate_index: c_idx,
                sl_buffer_atr: sl_buf,
                horizon_bars,
                target_r,
                full_kpis,
                is_kpis,
                oos_kpis,
                walk_forward_efficiency_pct: wfe_pct,
                deflated_sharpe_ratio: dsr,
                hedge,
            });
        }

        candidates.sort_by(|a, b| {
            match b
                .oos_kpis
                .net_avg_realized_r
                .cmp(&a.oos_kpis.net_avg_realized_r)
            {
                Ordering::Equal => match b
                    .full_kpis
                    .net_avg_realized_r
                    .cmp(&a.full_kpis.net_avg_realized_r)
                {
                    Ordering::Equal => a
                        .full_kpis
                        .stop_out_rate_pct
                        .cmp(&b.full_kpis.stop_out_rate_pct),
                    other => other,
                },
                other => other,
            }
        });

        let winner = candidates[0].clone();
        Ok(ParameterSweepReport {
            strategy_id: req.base.strategy_id.clone(),
            trial_count,
            candidates,
            winner,
        })
    }
}

/// Deterministic `Decimal` Deflated Sharpe Ratio proxy subtracting multiple-testing penalty
/// proportional to `sqrt(trial_count - 1) * 0.12`.
fn compute_deflated_sharpe(sharpe: Decimal, trial_count: usize) -> Decimal {
    if trial_count <= 1 {
        return sharpe.round_dp(2);
    }
    let excess_trials = Decimal::from((trial_count - 1) as u64);
    let penalty = decimal_sqrt(excess_trials) * Decimal::new(12, 2);
    (sharpe - penalty).round_dp(2)
}
