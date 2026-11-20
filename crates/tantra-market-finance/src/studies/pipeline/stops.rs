//! Primitive 5: Generic stop-loss & exit simulation policies (`OuterBufferStopPolicy`).

use rust_decimal::Decimal;

use super::super::decimal_sqrt;
use super::super::returns::mean_and_sample_std;
use super::super::schedule::StudyAssetClass;
use super::super::spreads::{RollBasisMode, SpreadCostConfig, estimate_statutory_roundtrip_bps};
use super::types::{
    EntrySignal, PenetrationDistribution, SignalDirection, SimulatedTrade, StopExitPolicy,
    StopPolicyMode, StopSimInput, StrategyKpis, StudyBar,
};
use super::zones::nearest_rank_percentile;

/// Generic stop-loss and trade exit simulator (`TightCenterLine`, `BandOuterEdge`,
/// `OuterAtrBuffer`, `PercentileShield`, `FixedHorizon`, `PartialAndTrail`, `TimeStop`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OuterBufferStopPolicy {
    mode: StopPolicyMode,
}

impl OuterBufferStopPolicy {
    /// Constructs a new stop/exit policy with `mode`.
    pub fn new(mode: StopPolicyMode) -> Self {
        Self { mode }
    }

    /// Computes initial stop-loss price matching `stop-hunt-buffer-study.ts` stop geometry.
    pub fn compute_stop_for_mode(mode: StopPolicyMode, signal: &EntrySignal) -> Decimal {
        let min_floor = Decimal::new(5, 2);
        let atr = signal.atr;
        let raw = match mode {
            StopPolicyMode::TightCenterLine => {
                let offset = Decimal::new(15, 2) * atr;
                match signal.direction {
                    SignalDirection::Long => (signal.entry_price - offset).min(signal.band_center),
                    SignalDirection::Short => (signal.entry_price + offset).max(signal.band_center),
                }
            }
            StopPolicyMode::BandOuterEdge => {
                let offset = Decimal::new(20, 2) * atr;
                match signal.direction {
                    SignalDirection::Long => (signal.entry_price - offset).min(signal.band_low),
                    SignalDirection::Short => (signal.entry_price + offset).max(signal.band_high),
                }
            }
            StopPolicyMode::OuterAtrBuffer { buffer_atr }
            | StopPolicyMode::PercentileShield { buffer_atr } => {
                let b = buffer_atr.max(Decimal::ZERO);
                let min_entry_mult =
                    (Decimal::new(20, 2) + b * Decimal::new(20, 2)).min(Decimal::new(40, 2));
                match signal.direction {
                    SignalDirection::Long => {
                        (signal.entry_price - min_entry_mult * atr).min(signal.band_low - b * atr)
                    }
                    SignalDirection::Short => {
                        (signal.entry_price + min_entry_mult * atr).max(signal.band_high + b * atr)
                    }
                }
            }
            StopPolicyMode::FixedHorizon | StopPolicyMode::TimeStop { .. } => {
                // Wide catastrophe shield so horizon exits govern outcomes cleanly
                let b = Decimal::from(2);
                match signal.direction {
                    SignalDirection::Long => signal.band_low - b * atr,
                    SignalDirection::Short => signal.band_high + b * atr,
                }
            }
            StopPolicyMode::PartialAndTrail { .. } => {
                let b = Decimal::new(25, 2);
                match signal.direction {
                    SignalDirection::Long => (signal.entry_price - Decimal::new(25, 2) * atr)
                        .min(signal.band_low - b * atr),
                    SignalDirection::Short => (signal.entry_price + Decimal::new(25, 2) * atr)
                        .max(signal.band_high + b * atr),
                }
            }
        };
        raw.max(min_floor).round_dp(2)
    }

    /// Computes canonical per-trade roundtrip friction in basis points from `spread_cost`.
    pub(crate) fn friction_bps(
        symbol: &str,
        asset_class: StudyAssetClass,
        spread_cost: &SpreadCostConfig,
    ) -> Decimal {
        match spread_cost.mode {
            RollBasisMode::GrossSpot => Decimal::ZERO,
            RollBasisMode::IncludeRollBasis => {
                (-spread_cost.futures_roll_basis_bps).max(Decimal::ZERO)
            }
            RollBasisMode::NetAfterSpreadAndCosts => {
                let statutory = if spread_cost.include_statutory_costs {
                    estimate_statutory_roundtrip_bps(symbol, asset_class)
                } else {
                    Decimal::ZERO
                };
                spread_cost.spread_slippage_bps.max(Decimal::ZERO) + statutory
                    - spread_cost.futures_roll_basis_bps
            }
        }
    }
}

impl StopExitPolicy for OuterBufferStopPolicy {
    fn policy_id(&self) -> String {
        match self.mode {
            StopPolicyMode::TightCenterLine => "tight_center".to_string(),
            StopPolicyMode::BandOuterEdge => "outer_edge_0".to_string(),
            StopPolicyMode::OuterAtrBuffer { buffer_atr } => {
                if buffer_atr.is_zero() {
                    "outer_edge_0".to_string()
                } else {
                    let bp = (buffer_atr * Decimal::from(100)).round_dp(0);
                    format!("outer_{:03}_atr", bp)
                }
            }
            StopPolicyMode::PercentileShield { buffer_atr } => {
                let bp = (buffer_atr * Decimal::from(100)).round_dp(0);
                format!("percentile_shield_{:03}_atr", bp)
            }
            StopPolicyMode::FixedHorizon => "fixed_horizon".to_string(),
            StopPolicyMode::PartialAndTrail { .. } => "partial_and_trail".to_string(),
            StopPolicyMode::TimeStop { bars } => format!("time_stop_{}b", bars),
        }
    }

    fn mode(&self) -> StopPolicyMode {
        self.mode
    }

    fn stop_price(&self, signal: &EntrySignal) -> Decimal {
        Self::compute_stop_for_mode(self.mode, signal)
    }

    fn simulate_trades(&self, input: &StopSimInput<'_>) -> (Vec<SimulatedTrade>, StrategyKpis) {
        let bars = input.bars;
        let signals = input.signals;
        if bars.is_empty() || signals.is_empty() {
            return (Vec::new(), StrategyKpis::default());
        }

        let eff_horizon = match self.mode {
            StopPolicyMode::TimeStop { bars: t_bars } => t_bars.max(1),
            _ => input.horizon_bars.max(1),
        };
        let eff_target_r = input.target_r.max(Decimal::ONE);
        let deep_mode = StopPolicyMode::OuterAtrBuffer {
            buffer_atr: Decimal::new(50, 2),
        };

        // Precompute deep-shield survivors to detect false stop-hunt traps cleanly
        let deep_survived_win: Vec<bool> = signals
            .iter()
            .map(|s| {
                let deep_sl = Self::compute_stop_for_mode(deep_mode, s);
                let end_idx = (s.bar_index + eff_horizon).min(bars.len() - 1);
                for b in bars.iter().take(end_idx + 1).skip(s.bar_index + 1) {
                    let hit = match s.direction {
                        SignalDirection::Long => b.low <= deep_sl,
                        SignalDirection::Short => b.high >= deep_sl,
                    };
                    if hit {
                        return false;
                    }
                }
                let final_close = bars[end_idx].close;
                match s.direction {
                    SignalDirection::Long => final_close > s.entry_price,
                    SignalDirection::Short => final_close < s.entry_price,
                }
            })
            .collect();

        let friction_bps = Self::friction_bps(input.symbol, input.asset_class, input.spread_cost);
        let ten_k = Decimal::from(10_000);
        let two = Decimal::from(2);

        let mut trades = Vec::with_capacity(signals.len());
        for (idx, s) in signals.iter().enumerate() {
            if s.bar_index >= bars.len() {
                continue;
            }
            let sl = self.stop_price(s);
            let one_r = (s.entry_price - sl)
                .abs()
                .max(Decimal::new(1, 1) * s.atr)
                .max(Decimal::new(1, 2));
            let target_2r = match s.direction {
                SignalDirection::Long => s.entry_price + two * one_r,
                SignalDirection::Short => s.entry_price - two * one_r,
            };
            let primary_target = match s.direction {
                SignalDirection::Long => s.entry_price + eff_target_r * one_r,
                SignalDirection::Short => s.entry_price - eff_target_r * one_r,
            };
            let end_idx = (s.bar_index + eff_horizon).min(bars.len() - 1);

            let (partial_r, runner_r) = match self.mode {
                StopPolicyMode::PartialAndTrail {
                    partial_r,
                    runner_r,
                } => (Some(partial_r), Some(runner_r)),
                _ => (None, None),
            };

            let mut active_sl = sl;
            let mut partial_taken = false;
            let mut stopped = false;
            let mut hit_2r = false;
            let mut hit_target = false;
            let mut exit_idx = end_idx;
            let mut exit_px = bars[end_idx].close;

            for (j, b) in bars
                .iter()
                .enumerate()
                .take(end_idx + 1)
                .skip(s.bar_index + 1)
            {
                match s.direction {
                    SignalDirection::Long => {
                        if b.low <= active_sl {
                            stopped = true;
                            exit_idx = j;
                            exit_px = active_sl;
                            break;
                        }
                        if b.high >= target_2r {
                            hit_2r = true;
                        }
                        if let (Some(pr), Some(rr)) = (partial_r, runner_r) {
                            if !partial_taken && b.high >= s.entry_price + pr * one_r {
                                partial_taken = true;
                                active_sl = s.entry_price; // move to breakeven
                            }
                            if b.high >= s.entry_price + rr * one_r {
                                hit_target = true;
                                exit_idx = j;
                                exit_px = s.entry_price + rr * one_r;
                                break;
                            }
                        } else if b.high >= primary_target {
                            hit_target = true;
                            exit_idx = j;
                            exit_px = primary_target;
                            break;
                        }
                    }
                    SignalDirection::Short => {
                        if b.high >= active_sl {
                            stopped = true;
                            exit_idx = j;
                            exit_px = active_sl;
                            break;
                        }
                        if b.low <= target_2r {
                            hit_2r = true;
                        }
                        if let (Some(pr), Some(rr)) = (partial_r, runner_r) {
                            if !partial_taken && b.low <= s.entry_price - pr * one_r {
                                partial_taken = true;
                                active_sl = s.entry_price;
                            }
                            if b.low <= s.entry_price - rr * one_r {
                                hit_target = true;
                                exit_idx = j;
                                exit_px = s.entry_price - rr * one_r;
                                break;
                            }
                        } else if b.low <= primary_target {
                            hit_target = true;
                            exit_idx = j;
                            exit_px = primary_target;
                            break;
                        }
                    }
                }
            }

            let gross_r = if stopped {
                if partial_taken {
                    // Half closed at partial_r, half stopped at breakeven
                    (partial_r.unwrap_or(two) * Decimal::new(5, 1)).round_dp(4)
                } else {
                    -Decimal::ONE
                }
            } else if hit_target {
                if let (Some(pr), Some(rr)) = (partial_r, runner_r) {
                    ((pr + rr) * Decimal::new(5, 1)).round_dp(4)
                } else {
                    eff_target_r.round_dp(4)
                }
            } else {
                let raw_r = match s.direction {
                    SignalDirection::Long => (exit_px - s.entry_price) / one_r,
                    SignalDirection::Short => (s.entry_price - exit_px) / one_r,
                };
                if partial_taken {
                    ((partial_r.unwrap_or(two) + raw_r) * Decimal::new(5, 1)).round_dp(4)
                } else {
                    raw_r.round_dp(4)
                }
            };

            let friction_r = if friction_bps.is_zero() {
                Decimal::ZERO
            } else {
                ((s.entry_price * friction_bps / ten_k) / one_r).round_dp(4)
            };
            let net_r = (gross_r - friction_r).round_dp(4);
            let false_sweep_trap = stopped && !partial_taken && deep_survived_win[idx];

            trades.push(SimulatedTrade {
                entry_bar_index: s.bar_index,
                exit_bar_index: exit_idx,
                direction: s.direction,
                entry_price: s.entry_price,
                stop_price: sl,
                target_price: primary_target,
                exit_price: exit_px,
                one_r: one_r.round_dp(4),
                stopped_out: stopped && !partial_taken,
                false_sweep_trap,
                hit_2r,
                hit_target,
                gross_r,
                net_r,
            });
        }

        let kpis = summarize_trades_kpis(&trades);
        (trades, kpis)
    }
}

/// Aggregates [`StrategyKpis`] from a slice of [`SimulatedTrade`].
pub(crate) fn summarize_trades_kpis(trades: &[SimulatedTrade]) -> StrategyKpis {
    if trades.is_empty() {
        return StrategyKpis::default();
    }
    let n = trades.len();
    let n_dec = Decimal::from(n as u64);
    let hundred = Decimal::from(100);

    let mut stopped_out_count = 0usize;
    let mut false_sweep_stop_count = 0usize;
    let mut win_count = 0usize;
    let mut hit_2r_count = 0usize;
    let mut gross_win_r = Decimal::ZERO;
    let mut gross_loss_r = Decimal::ZERO;
    let mut net_win_r = Decimal::ZERO;
    let mut net_loss_r = Decimal::ZERO;
    let mut total_gross_r = Decimal::ZERO;
    let mut total_net_r = Decimal::ZERO;

    let mut equity = Decimal::ZERO;
    let mut peak = Decimal::ZERO;
    let mut max_dd = Decimal::ZERO;
    let mut r_values = Vec::with_capacity(n);

    for t in trades {
        if t.stopped_out {
            stopped_out_count += 1;
        }
        if t.false_sweep_trap {
            false_sweep_stop_count += 1;
        }
        if t.hit_2r {
            hit_2r_count += 1;
        }
        if t.gross_r > Decimal::ZERO {
            win_count += 1;
            gross_win_r += t.gross_r;
        } else {
            gross_loss_r += t.gross_r.abs();
        }
        if t.net_r > Decimal::ZERO {
            net_win_r += t.net_r;
        } else {
            net_loss_r += t.net_r.abs();
        }
        total_gross_r += t.gross_r;
        total_net_r += t.net_r;
        r_values.push(t.net_r);

        equity += t.net_r;
        if equity > peak {
            peak = equity;
        }
        let dd = peak - equity;
        if dd > max_dd {
            max_dd = dd;
        }
    }

    let (mean_r, std_r) = mean_and_sample_std(&r_values);
    let sharpe_ratio = if std_r > Decimal::ZERO {
        (mean_r / std_r * decimal_sqrt(n_dec)).round_dp(2)
    } else {
        Decimal::ZERO
    };

    let pf = if gross_loss_r > Decimal::ZERO {
        (gross_win_r / gross_loss_r).round_dp(2)
    } else if gross_win_r > Decimal::ZERO {
        Decimal::from(99)
    } else {
        Decimal::ZERO
    };

    let net_pf = if net_loss_r > Decimal::ZERO {
        (net_win_r / net_loss_r).round_dp(2)
    } else if net_win_r > Decimal::ZERO {
        Decimal::from(99)
    } else {
        Decimal::ZERO
    };

    StrategyKpis {
        trades: n,
        stopped_out_count,
        stop_out_rate_pct: (Decimal::from(stopped_out_count as u64) / n_dec * hundred).round_dp(2),
        false_sweep_stop_count,
        false_sweep_stop_rate_pct: (Decimal::from(false_sweep_stop_count as u64) / n_dec * hundred)
            .round_dp(2),
        win_count,
        win_rate_pct: (Decimal::from(win_count as u64) / n_dec * hundred).round_dp(2),
        target_2r_hit_rate_pct: (Decimal::from(hit_2r_count as u64) / n_dec * hundred).round_dp(2),
        avg_realized_r: (total_gross_r / n_dec).round_dp(3),
        net_avg_realized_r: (total_net_r / n_dec).round_dp(3),
        profit_factor: pf,
        net_profit_factor: net_pf,
        sharpe_ratio,
        max_drawdown_r: max_dd.round_dp(2),
    }
}

/// Computes empirical penetration percentiles beyond zone outer edges across `signals`.
pub(crate) fn compute_penetration_distribution(
    bars: &[StudyBar],
    signals: &[EntrySignal],
    horizon_bars: usize,
) -> PenetrationDistribution {
    if bars.is_empty() || signals.is_empty() {
        return PenetrationDistribution::default();
    }
    let h_bars = horizon_bars.max(1);
    let hundred = Decimal::from(100);
    let mut pens = Vec::with_capacity(signals.len());
    let mut outer_touch_count = 0usize;
    let mut outer_recover_count = 0usize;

    for s in signals {
        if s.bar_index >= bars.len() || s.atr <= Decimal::ZERO {
            continue;
        }
        let end_idx = (s.bar_index + h_bars).min(bars.len() - 1);
        let mut max_beyond = Decimal::ZERO;
        let mut breached_outer = false;

        for b in bars.iter().take(end_idx + 1).skip(s.bar_index) {
            let excess = match s.direction {
                SignalDirection::Long => ((s.band_low - b.low) / s.atr).max(Decimal::ZERO),
                SignalDirection::Short => ((b.high - s.band_high) / s.atr).max(Decimal::ZERO),
            };
            if excess > Decimal::ZERO {
                breached_outer = true;
            }
            if excess > max_beyond {
                max_beyond = excess;
            }
        }

        let horizon_close = bars[end_idx].close;
        let recovered = match s.direction {
            SignalDirection::Long => horizon_close > s.entry_price,
            SignalDirection::Short => horizon_close < s.entry_price,
        };
        if breached_outer {
            outer_touch_count += 1;
            if recovered {
                outer_recover_count += 1;
            }
        }
        pens.push(max_beyond);
    }

    if pens.is_empty() {
        return PenetrationDistribution::default();
    }

    pens.sort();
    let recover_pct = if outer_touch_count > 0 {
        (Decimal::from(outer_recover_count as u64) / Decimal::from(outer_touch_count as u64)
            * hundred)
            .round_dp(2)
    } else {
        Decimal::ZERO
    };

    PenetrationDistribution {
        sample_count: pens.len(),
        p50_atr: nearest_rank_percentile(&pens, 50).round_dp(2),
        p75_atr: nearest_rank_percentile(&pens, 75).round_dp(2),
        p90_atr: nearest_rank_percentile(&pens, 90).round_dp(2),
        p95_atr: nearest_rank_percentile(&pens, 95).round_dp(2),
        max_atr: pens.last().copied().unwrap_or(Decimal::ZERO).round_dp(2),
        pct_recovering_after_outer_touch: recover_pct,
    }
}
