//! Pure `Decimal` Multi-Leg F&O Options Overlay, Strike Lab, Analog Forecasting,
//! and Cumulative Abnormal Return (`CAR[-20d, +20d]`) Event Study engine (`wasm32`-ready).
//!
//! Zero floating-point arithmetic (`f32`/`f64`) inside calculations; all option
//! strike rounding, spread/statutory cost deductions, quantile fans, and event-window
//! drift metrics execute deterministically in [`rust_decimal::Decimal`].

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tantra_core::Exchange;

use crate::india::costs::segments::{SegmentProduct, turnover_fee_rate};

use super::decimal_sqrt;
use super::distributions::empirical_quantile;
use super::returns::mean_and_sample_std;
use super::schedule::{OhlcvBar, SlicedPeriod, StudyAssetClass};
use super::spreads::resolve_exchange_for_costs;
use super::streaks_markov::{MarkovState, StreakDirection};

// ─────────────────────────────────────────────────────────────────────────────
// Part A: Multi-Leg F&O Options Overlay & Expiry Strike Lab
// ─────────────────────────────────────────────────────────────────────────────

/// Option overlay strategy family evaluated in Tab 8 & Tab 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OptionOverlayStrategy {
    /// Long 1× underlying + Short 1× OTM Call at empirical target probability strike.
    #[default]
    CoveredCall,
    /// Defined-risk Bull Call vertical spread (Long ATM CE + Short OTM CE).
    BullCallSpread,
    /// Defined-risk Bear Put vertical spread (Long ATM PE + Short OTM PE).
    BearPutSpread,
    /// Four-leg range-bound Iron Condor (Short OTM CE/PE + Long protective wings).
    IronCondor,
    /// Calendar time-spread roll proxy (Short near-cycle ATM CE + Long next-cycle ATM CE).
    CalendarSpread,
}

/// Optional live/snapshot option contract quote passed from `ChainReader`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveOptionStrikeQuote {
    /// `"CE"` or `"PE"`.
    pub option_type: String,
    /// Strike price.
    pub strike: Decimal,
    /// Contract expiry date.
    pub expiry: NaiveDate,
    /// Contract lot size.
    pub lot_size: Decimal,
    /// Best bid price (`None` when top-of-book quote unavailable).
    pub bid: Option<Decimal>,
    /// Best ask price (`None` when top-of-book quote unavailable).
    pub ask: Option<Decimal>,
    /// Last traded price if available.
    pub last_price: Option<Decimal>,
    /// Latest open interest if available.
    pub open_interest: Option<Decimal>,
}

/// Parameters controlling Multi-Leg Options Overlay & Strike Lab simulation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionsOverlayConfig {
    /// Instrument symbol/ID.
    pub symbol: String,
    /// Asset class context.
    pub asset_class: StudyAssetClass,
    /// Selected option strategy.
    pub strategy: OptionOverlayStrategy,
    /// Target non-exceedance probability (`55..=98`, e.g. `80` = 80% OTM wing probability).
    pub target_short_prob_pct: Decimal,
    /// Wing width percentage separating short strike from long protective wing (`0.5..=15.0`).
    pub wing_width_pct: Decimal,
    /// Instrument lot size (`>= 1`).
    pub lot_size: Decimal,
    /// Exchange price tick size (`>= 0.01`).
    pub tick_size: Decimal,
    /// Fallback round-trip bid-ask spread slippage in bps (`0..=500`) when live depth is absent.
    pub spread_slippage_bps: Decimal,
    /// Whether to deduct statutory Indian options turnover charges (STT, exchange fee, GST, stamp, SEBI).
    pub include_statutory_costs: bool,
    /// Latest live spot override (`None` falls back to last bar close).
    pub live_spot_override: Option<Decimal>,
}

/// Single leg description inside an option overlay / spread structure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionLegSpec {
    /// Strategy leg label (`"Long Underlying"`, `"Short OTM Call"`, etc.).
    pub role: String,
    /// `"BUY"` or `"SELL"`.
    pub side: String,
    /// `"SPOT"`, `"CE"`, or `"PE"`.
    pub option_type: String,
    /// Selected strike price (or spot entry for underlying leg).
    pub strike: Decimal,
    /// Estimated/theoretical cycle fair value (% of spot for options, `100` normalized for spot).
    pub fair_value_pct: Decimal,
    /// Quoted live bid price when option chain is loaded.
    pub bid: Option<Decimal>,
    /// Quoted live ask price when option chain is loaded.
    pub ask: Option<Decimal>,
    /// Quoted bid-ask spread in price units (`ask - bid`).
    pub spread_abs: Option<Decimal>,
    /// Quoted bid-ask spread in basis points relative to mid price.
    pub spread_bps: Option<Decimal>,
    /// Effective leg price after crossing bid/ask or configured slippage.
    pub execution_price: Decimal,
}

/// Option strike liquidity & empirical exceedance row for Tab 9 (`Expiry Strike Lab`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrikeLabRow {
    /// `"CE"` or `"PE"`.
    pub option_type: String,
    /// Strike price.
    pub strike: Decimal,
    /// Option expiry date when sourced from live chain.
    pub expiry: Option<NaiveDate>,
    /// Best bid price (`None` when live option chain is offline/empty).
    pub bid: Option<Decimal>,
    /// Best ask price (`None` when live option chain is offline/empty).
    pub ask: Option<Decimal>,
    /// Midpoint `(bid + ask) / 2` when both sides exist.
    pub mid: Option<Decimal>,
    /// Quoted bid-ask spread (`ask - bid`).
    pub spread_abs: Option<Decimal>,
    /// Quoted bid-ask spread in bps `(ask - bid) / mid * 10000`.
    pub spread_bps: Option<Decimal>,
    /// Liquidity tier: `"tight"` (`<= 40 bps`), `"moderate"` (`<= 120 bps`), `"wide"`, or `"unquoted"`.
    pub liquidity_tier: String,
    /// Latest open interest when available.
    pub open_interest: Option<Decimal>,
    /// Empirical probability (%) spot closes beyond this strike (`>= strike` for CE, `<= strike` for PE).
    pub exceedance_prob_pct: Decimal,
    /// Strike moneyness relative to spot `(strike - spot) / spot * 100`.
    pub moneyness_pct: Decimal,
}

/// Period-by-period equity curve comparison point for Buy-and-Hold vs Options Overlay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayEquityPoint {
    /// Period end date.
    pub date: NaiveDate,
    /// Underlying buy-and-hold cumulative return (%).
    pub buy_hold_cum_pct: Decimal,
    /// Strategy overlay cumulative gross return (%) before spread & statutory costs.
    pub overlay_gross_cum_pct: Decimal,
    /// Strategy overlay cumulative net return (%) after spread & statutory costs.
    pub overlay_net_cum_pct: Decimal,
    /// Single-period net strategy payoff (% of underlying reference).
    pub period_payoff_pct: Decimal,
}

/// Complete Multi-Leg Options Overlay & Strike Lab analysis bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionsOverlayReport {
    /// Evaluated strategy family.
    pub strategy: OptionOverlayStrategy,
    /// Reference underlying spot price.
    pub spot_price: Decimal,
    /// Empirical annualized realized volatility (%) derived from period returns.
    pub empirical_rv_pct: Decimal,
    /// Implied volatility proxy (%) from live ATM option quotes (`None` when no live chain).
    pub quoted_iv_proxy_pct: Option<Decimal>,
    /// Volatility risk premium (`IV - RV` in % points) when live chain is present.
    pub iv_minus_rv_pct: Option<Decimal>,
    /// `true` when at least one live option quote with bid/ask or last price was matched.
    pub live_chain_available: bool,
    /// Count of live option contracts inspected.
    pub live_contract_count: usize,
    /// Nearest live option expiry date inspected, if any.
    pub nearest_expiry: Option<NaiveDate>,
    /// Configured strategy legs with quoted bid/ask & execution pricing.
    pub legs: Vec<OptionLegSpec>,
    /// Net option premium credit (`> 0`) or debit (`< 0`) per cycle as % of spot before friction.
    pub net_premium_pct: Decimal,
    /// Bid-ask spread execution drag (% of spot) per cycle.
    pub total_spread_drag_pct: Decimal,
    /// Statutory derivative turnover fee drag (% of spot) per cycle.
    pub statutory_cost_pct: Decimal,
    /// Net option credit/debit (% of spot) after spread and statutory fees.
    pub net_after_friction_pct: Decimal,
    /// Maximum theoretical/structure profit per cycle (% of spot), if capped.
    pub max_profit_pct: Option<Decimal>,
    /// Maximum theoretical/structure loss per cycle (% of spot, <= 0), if bounded.
    pub max_loss_pct: Option<Decimal>,
    /// Empirical probability of profit (%) across historical cycles after friction.
    pub probability_of_profit_pct: Decimal,
    /// Final compounded buy-and-hold cumulative return (%).
    pub buy_hold_cum_pct: Decimal,
    /// Final compounded gross overlay cumulative return (%).
    pub overlay_gross_cum_pct: Decimal,
    /// Final compounded net overlay cumulative return (%).
    pub overlay_net_cum_pct: Decimal,
    /// Maximum peak-to-trough drawdown (%) of net overlay curve.
    pub overlay_max_drawdown_pct: Decimal,
    /// Maximum peak-to-trough drawdown (%) of underlying buy-and-hold curve.
    pub buy_hold_max_drawdown_pct: Decimal,
    /// Count of winning net periods (`> 0`).
    pub win_periods: usize,
    /// Total historical periods simulated.
    pub total_periods: usize,
    /// Equity comparison series.
    pub equity_curve: Vec<OverlayEquityPoint>,
    /// Strike-level bid-ask liquidity & empirical exceedance lab rows.
    pub strike_lab: Vec<StrikeLabRow>,
    /// Honest data-source & quote availability status note.
    pub availability_note: String,
}

fn strike_increment_for_spot(spot: Decimal, tick: Decimal) -> Decimal {
    let raw = if spot >= Decimal::from(20_000) {
        Decimal::from(100)
    } else if spot >= Decimal::from(5_000) {
        Decimal::from(50)
    } else if spot >= Decimal::from(1_000) {
        Decimal::from(20)
    } else if spot >= Decimal::from(200) {
        Decimal::from(5)
    } else {
        Decimal::ONE
    };
    snap_to_step(raw, tick.max(Decimal::new(1, 2))).max(tick)
}

fn snap_to_step(val: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return val.round_dp(2);
    }
    ((val / step).round() * step).round_dp(4)
}

fn compute_max_drawdown_pct(cum_returns_pct: &[Decimal]) -> Decimal {
    let hundred = Decimal::from(100);
    let mut peak = hundred;
    let mut max_dd = Decimal::ZERO;
    for r in cum_returns_pct {
        let equity = hundred + *r;
        if equity > peak {
            peak = equity;
        }
        if peak > Decimal::ZERO {
            let dd = ((peak - equity) / peak * hundred).round_dp(4);
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd.round_dp(2)
}

fn empirical_option_fair_pct(
    period_returns: &[Decimal],
    strike_rel_pct: Decimal,
    is_call: bool,
) -> Decimal {
    if period_returns.is_empty() {
        return Decimal::new(50, 2); // 0.50% floor
    }
    let count = Decimal::from(period_returns.len());
    let mut intrinsic_sum = Decimal::ZERO;
    for r in period_returns {
        let payoff = if is_call {
            (*r - strike_rel_pct).max(Decimal::ZERO)
        } else {
            (strike_rel_pct - *r).max(Decimal::ZERO)
        };
        intrinsic_sum += payoff;
    }
    let expected = (intrinsic_sum / count).round_dp(4);
    // Add modest time-value premium floor above pure intrinsic expectation.
    expected.max(Decimal::new(15, 2)).round_dp(4)
}

fn estimate_option_statutory_pct(
    exchange_or_symbol: &str,
    asset_class: StudyAssetClass,
    premium_turnover_pct: Decimal,
    leg_count: usize,
) -> Decimal {
    if premium_turnover_pct <= Decimal::ZERO || leg_count == 0 {
        return Decimal::ZERO;
    }
    let ex = resolve_exchange_for_costs(exchange_or_symbol, asset_class);
    let gst_mult = Decimal::new(118, 2);
    let sebi_one_side = Decimal::new(1, 6);
    let exch = match ex {
        Exchange::Mcx => turnover_fee_rate(Exchange::Mcx, SegmentProduct::Options)
            .unwrap_or_else(|| Decimal::new(418, 6)),
        Exchange::Cds => turnover_fee_rate(Exchange::Cds, SegmentProduct::Options)
            .unwrap_or_else(|| Decimal::new(311, 6)),
        _ => turnover_fee_rate(Exchange::Nfo, SegmentProduct::Options)
            .unwrap_or_else(|| Decimal::new(3553, 7)),
    };
    // Approximate Indian options statutory rate across entry+exit on premium:
    // Sell-side STT on options premium (0.1%) + buy-side stamp (0.003%) + 2*(exch+SEBI)*GST
    let stt_sell = Decimal::new(1, 3);
    let stamp_buy = Decimal::new(3, 5);
    let roundtrip_rate =
        stt_sell + stamp_buy + (exch + sebi_one_side) * Decimal::from(2) * gst_mult;
    // Also add nominal per-leg exchange/clearing friction floor so multi-leg spreads reflect leg count.
    let per_leg_floor = Decimal::new(5, 3) * Decimal::from(leg_count as u64); // 0.005% per leg
    (premium_turnover_pct * roundtrip_rate + per_leg_floor).round_dp(4)
}

fn empirical_exceedance_prob(
    sorted_returns: &[Decimal],
    rel_pct: Decimal,
    is_call: bool,
) -> Decimal {
    if sorted_returns.is_empty() {
        return Decimal::from(50);
    }
    let n = Decimal::from(sorted_returns.len());
    let hits = if is_call {
        sorted_returns.iter().filter(|r| **r >= rel_pct).count()
    } else {
        sorted_returns.iter().filter(|r| **r <= rel_pct).count()
    };
    (Decimal::from(hits) / n * Decimal::from(100)).round_dp(2)
}

fn quote_mid_spread(
    q: &LiveOptionStrikeQuote,
) -> (Option<Decimal>, Option<Decimal>, Option<Decimal>) {
    match (q.bid, q.ask) {
        (Some(b), Some(a)) if a >= b && a > Decimal::ZERO => {
            let mid = ((b + a) / Decimal::from(2)).round_dp(4);
            let spread = (a - b).round_dp(4);
            let bps = if mid > Decimal::ZERO {
                Some((spread / mid * Decimal::from(10_000)).round_dp(2))
            } else {
                None
            };
            (Some(mid), Some(spread), bps)
        }
        _ => (q.last_price, None, None),
    }
}

fn liquidity_tier_for_bps(spread_bps: Option<Decimal>) -> &'static str {
    match spread_bps {
        Some(bps) if bps <= Decimal::from(40) => "tight",
        Some(bps) if bps <= Decimal::from(120) => "moderate",
        Some(_) => "wide",
        None => "unquoted",
    }
}

/// Evaluates Multi-Leg F&O Options Overlay & Expiry Strike Lab across `periods` & live chain quotes.
pub fn evaluate_options_overlay(
    periods: &[SlicedPeriod],
    live_quotes: &[LiveOptionStrikeQuote],
    config: &OptionsOverlayConfig,
) -> OptionsOverlayReport {
    let hundred = Decimal::from(100);
    let spot = config
        .live_spot_override
        .filter(|p| *p > Decimal::ZERO)
        .or_else(|| periods.last().map(|p| p.exit_price))
        .unwrap_or(Decimal::from(100));
    let tick = config.tick_size.max(Decimal::new(1, 2));
    let strike_step = strike_increment_for_spot(spot, tick);

    let returns: Vec<Decimal> = periods.iter().map(|p| p.return_pct).collect();
    let mut sorted = returns.clone();
    sorted.sort();

    let (_mean, period_std) = mean_and_sample_std(&returns);
    let avg_days = if periods.is_empty() {
        Decimal::from(7)
    } else {
        let total_days: i64 = periods
            .iter()
            .map(|p| (p.end_date - p.start_date).num_days().max(1))
            .sum();
        (Decimal::from(total_days) / Decimal::from(periods.len())).max(Decimal::ONE)
    };
    let periods_per_year = (Decimal::from(365) / avg_days).max(Decimal::ONE);
    let empirical_rv_pct = (period_std * decimal_sqrt(periods_per_year)).round_dp(2);

    let target_prob = config
        .target_short_prob_pct
        .clamp(Decimal::from(55), Decimal::from(98));
    let wing_width = config
        .wing_width_pct
        .clamp(Decimal::new(5, 1), Decimal::from(15));

    let short_call_q = empirical_quantile(&sorted, target_prob).max(Decimal::new(5, 1));
    let short_put_q = empirical_quantile(&sorted, hundred - target_prob).min(Decimal::new(-5, 1));

    let atm_strike = snap_to_step(spot, strike_step).max(tick);
    let short_call_strike =
        snap_to_step(spot * (Decimal::ONE + short_call_q / hundred), strike_step)
            .max(atm_strike + strike_step);
    let long_call_wing_strike = snap_to_step(
        short_call_strike + (spot * wing_width / hundred).max(strike_step),
        strike_step,
    )
    .max(short_call_strike + strike_step);
    let short_put_strike = snap_to_step(spot * (Decimal::ONE + short_put_q / hundred), strike_step)
        .min(atm_strike - strike_step)
        .max(strike_step);
    let long_put_wing_strike = snap_to_step(
        (short_put_strike - (spot * wing_width / hundred).max(strike_step)).max(strike_step),
        strike_step,
    )
    .min((short_put_strike - strike_step).max(strike_step));

    let atm_rel = Decimal::ZERO;
    let short_call_rel = if spot > Decimal::ZERO {
        ((short_call_strike - spot) / spot * hundred).round_dp(4)
    } else {
        short_call_q
    };
    let long_call_wing_rel = if spot > Decimal::ZERO {
        ((long_call_wing_strike - spot) / spot * hundred).round_dp(4)
    } else {
        short_call_q + wing_width
    };
    let short_put_rel = if spot > Decimal::ZERO {
        ((short_put_strike - spot) / spot * hundred).round_dp(4)
    } else {
        short_put_q
    };
    let long_put_wing_rel = if spot > Decimal::ZERO {
        ((long_put_wing_strike - spot) / spot * hundred).round_dp(4)
    } else {
        short_put_q - wing_width
    };

    // Inspect nearest expiry live quotes if provided.
    let nearest_expiry = live_quotes.iter().map(|q| q.expiry).min();
    let active_quotes: Vec<&LiveOptionStrikeQuote> = match nearest_expiry {
        Some(exp) => live_quotes.iter().filter(|q| q.expiry == exp).collect(),
        None => Vec::new(),
    };
    let quoted_count = active_quotes
        .iter()
        .filter(|q| q.bid.is_some() || q.ask.is_some() || q.last_price.is_some())
        .count();
    let live_chain_available = quoted_count > 0;

    // Match helper finding closest quoted strike on nearest expiry.
    let find_live_quote =
        |opt_type: &str, target_strike: Decimal| -> Option<&LiveOptionStrikeQuote> {
            active_quotes
                .iter()
                .copied()
                .filter(|q| q.option_type.eq_ignore_ascii_case(opt_type))
                .min_by_key(|q| (q.strike - target_strike).abs())
                .filter(|q| {
                    if spot > Decimal::ZERO {
                        ((q.strike - target_strike).abs() / spot) <= Decimal::new(5, 2)
                    } else {
                        true
                    }
                })
        };

    let build_option_leg = |role: &str,
                            side: &str,
                            opt_type: &str,
                            strike: Decimal,
                            fair_pct: Decimal|
     -> OptionLegSpec {
        let matched = find_live_quote(opt_type, strike);
        let theo_price = (spot * fair_pct / hundred).max(tick).round_dp(2);
        let half_slip_frac = config
            .spread_slippage_bps
            .clamp(Decimal::ZERO, Decimal::from(500))
            / Decimal::from(20_000);
        if let Some(q) = matched {
            let (mid_opt, spread_abs, spread_bps) = quote_mid_spread(q);
            let mid = mid_opt.unwrap_or(theo_price);
            let exec_price = if side == "BUY" {
                q.ask
                    .unwrap_or_else(|| (mid * (Decimal::ONE + half_slip_frac)).round_dp(2))
            } else {
                q.bid
                    .unwrap_or_else(|| (mid * (Decimal::ONE - half_slip_frac)).round_dp(2))
            };
            OptionLegSpec {
                role: role.to_string(),
                side: side.to_string(),
                option_type: opt_type.to_string(),
                strike: q.strike,
                fair_value_pct: if spot > Decimal::ZERO {
                    (mid / spot * hundred).round_dp(4)
                } else {
                    fair_pct
                },
                bid: q.bid,
                ask: q.ask,
                spread_abs,
                spread_bps,
                execution_price: exec_price.max(tick),
            }
        } else {
            let exec_price = if side == "BUY" {
                (theo_price * (Decimal::ONE + half_slip_frac)).round_dp(2)
            } else {
                (theo_price * (Decimal::ONE - half_slip_frac)).round_dp(2)
            };
            OptionLegSpec {
                role: role.to_string(),
                side: side.to_string(),
                option_type: opt_type.to_string(),
                strike,
                fair_value_pct: fair_pct.round_dp(4),
                bid: None,
                ask: None,
                spread_abs: None,
                spread_bps: None,
                execution_price: exec_price.max(tick),
            }
        }
    };

    let atm_call_fair = empirical_option_fair_pct(&returns, atm_rel, true);
    let short_call_fair = empirical_option_fair_pct(&returns, short_call_rel, true);
    let long_call_wing_fair = empirical_option_fair_pct(&returns, long_call_wing_rel, true)
        .min((short_call_fair * Decimal::new(6, 1)).round_dp(4));
    let atm_put_fair = empirical_option_fair_pct(&returns, atm_rel, false);
    let short_put_fair = empirical_option_fair_pct(&returns, short_put_rel, false);
    let long_put_wing_fair = empirical_option_fair_pct(&returns, long_put_wing_rel, false)
        .min((short_put_fair * Decimal::new(6, 1)).round_dp(4));

    let legs: Vec<OptionLegSpec> = match config.strategy {
        OptionOverlayStrategy::CoveredCall => vec![
            OptionLegSpec {
                role: "Long Underlying".to_string(),
                side: "BUY".to_string(),
                option_type: "SPOT".to_string(),
                strike: spot.round_dp(2),
                fair_value_pct: hundred,
                bid: None,
                ask: None,
                spread_abs: None,
                spread_bps: None,
                execution_price: spot.round_dp(2),
            },
            build_option_leg(
                "Short OTM Call",
                "SELL",
                "CE",
                short_call_strike,
                short_call_fair,
            ),
        ],
        OptionOverlayStrategy::BullCallSpread => vec![
            build_option_leg("Long ATM Call", "BUY", "CE", atm_strike, atm_call_fair),
            build_option_leg(
                "Short OTM Call",
                "SELL",
                "CE",
                short_call_strike,
                short_call_fair,
            ),
        ],
        OptionOverlayStrategy::BearPutSpread => vec![
            build_option_leg("Long ATM Put", "BUY", "PE", atm_strike, atm_put_fair),
            build_option_leg(
                "Short OTM Put",
                "SELL",
                "PE",
                short_put_strike,
                short_put_fair,
            ),
        ],
        OptionOverlayStrategy::IronCondor => vec![
            build_option_leg(
                "Short OTM Call",
                "SELL",
                "CE",
                short_call_strike,
                short_call_fair,
            ),
            build_option_leg(
                "Long Call Wing",
                "BUY",
                "CE",
                long_call_wing_strike,
                long_call_wing_fair,
            ),
            build_option_leg(
                "Short OTM Put",
                "SELL",
                "PE",
                short_put_strike,
                short_put_fair,
            ),
            build_option_leg(
                "Long Put Wing",
                "BUY",
                "PE",
                long_put_wing_strike,
                long_put_wing_fair,
            ),
        ],
        OptionOverlayStrategy::CalendarSpread => {
            let far_call_fair = (atm_call_fair * Decimal::new(14, 1)).round_dp(4);
            vec![
                build_option_leg(
                    "Short Near ATM Call",
                    "SELL",
                    "CE",
                    atm_strike,
                    atm_call_fair,
                ),
                build_option_leg("Long Next ATM Call", "BUY", "CE", atm_strike, far_call_fair),
            ]
        }
    };

    // Net option premium credit/debit & spread drag (% of spot).
    let option_legs: Vec<&OptionLegSpec> =
        legs.iter().filter(|l| l.option_type != "SPOT").collect();
    let mut net_premium_pct = Decimal::ZERO;
    let mut premium_turnover_pct = Decimal::ZERO;
    let mut spread_drag_pct = Decimal::ZERO;
    let fallback_slip_bps = config
        .spread_slippage_bps
        .clamp(Decimal::ZERO, Decimal::from(500));

    for leg in &option_legs {
        premium_turnover_pct += leg.fair_value_pct;
        if leg.side == "SELL" {
            net_premium_pct += leg.fair_value_pct;
        } else {
            net_premium_pct -= leg.fair_value_pct;
        }
        if let (Some(b), Some(a)) = (leg.bid, leg.ask)
            && a > b
            && spot > Decimal::ZERO
        {
            // Half-spread on entry + half-spread on exit/settlement
            spread_drag_pct += ((a - b) / spot * hundred).round_dp(4);
        } else {
            spread_drag_pct +=
                (leg.fair_value_pct * fallback_slip_bps / Decimal::from(10_000)).round_dp(4);
        }
    }

    let statutory_cost_pct = if config.include_statutory_costs {
        estimate_option_statutory_pct(
            &config.symbol,
            config.asset_class,
            premium_turnover_pct,
            option_legs.len(),
        )
    } else {
        Decimal::ZERO
    };

    let total_friction_pct = (spread_drag_pct + statutory_cost_pct).round_dp(4);
    let net_after_friction_pct = (net_premium_pct - total_friction_pct).round_dp(4);

    // Empirical historical payoff simulation across sliced periods.
    let mut bh_eq = Decimal::ONE;
    let mut gross_eq = Decimal::ONE;
    let mut net_eq = Decimal::ONE;
    let mut bh_series = Vec::with_capacity(periods.len());
    let mut net_series = Vec::with_capacity(periods.len());
    let mut equity_curve = Vec::with_capacity(periods.len());
    let mut win_periods = 0usize;

    for p in periods {
        let r = p.return_pct;
        let gross_period_pct = match config.strategy {
            OptionOverlayStrategy::CoveredCall => {
                let capped_call_intrinsic = (r - short_call_rel).max(Decimal::ZERO);
                (r + short_call_fair - capped_call_intrinsic).round_dp(4)
            }
            OptionOverlayStrategy::BullCallSpread => {
                let spread_width = (short_call_rel - atm_rel).max(Decimal::new(5, 1));
                let debit = (atm_call_fair - short_call_fair).max(Decimal::new(10, 2));
                let payoff = (r - atm_rel).max(Decimal::ZERO).min(spread_width);
                (payoff - debit).round_dp(4)
            }
            OptionOverlayStrategy::BearPutSpread => {
                let spread_width = (atm_rel - short_put_rel).max(Decimal::new(5, 1));
                let debit = (atm_put_fair - short_put_fair).max(Decimal::new(10, 2));
                let payoff = (atm_rel - r).max(Decimal::ZERO).min(spread_width);
                (payoff - debit).round_dp(4)
            }
            OptionOverlayStrategy::IronCondor => {
                let call_loss = ((r - short_call_rel).max(Decimal::ZERO))
                    .min((long_call_wing_rel - short_call_rel).max(Decimal::new(5, 1)));
                let put_loss = ((short_put_rel - r).max(Decimal::ZERO))
                    .min((short_put_rel - long_put_wing_rel).max(Decimal::new(5, 1)));
                (net_premium_pct - call_loss - put_loss).round_dp(4)
            }
            OptionOverlayStrategy::CalendarSpread => {
                // Gains if spot stays near ATM; loses theta advantage on large trend moves
                let move_drag =
                    (r.abs() * Decimal::new(35, 2)).min(atm_call_fair * Decimal::from(2));
                (atm_call_fair * Decimal::new(65, 2) - move_drag).round_dp(4)
            }
        };

        let net_period_pct = (gross_period_pct - total_friction_pct).round_dp(4);
        if net_period_pct > Decimal::ZERO {
            win_periods += 1;
        }

        bh_eq *= (Decimal::ONE + r / hundred).max(Decimal::new(1, 4));
        gross_eq *= (Decimal::ONE + gross_period_pct / hundred).max(Decimal::new(1, 4));
        net_eq *= (Decimal::ONE + net_period_pct / hundred).max(Decimal::new(1, 4));

        let bh_cum = ((bh_eq - Decimal::ONE) * hundred).round_dp(2);
        let gross_cum = ((gross_eq - Decimal::ONE) * hundred).round_dp(2);
        let net_cum = ((net_eq - Decimal::ONE) * hundred).round_dp(2);

        bh_series.push(bh_cum);
        net_series.push(net_cum);
        equity_curve.push(OverlayEquityPoint {
            date: p.end_date,
            buy_hold_cum_pct: bh_cum,
            overlay_gross_cum_pct: gross_cum,
            overlay_net_cum_pct: net_cum,
            period_payoff_pct: net_period_pct.round_dp(2),
        });
    }

    let (max_profit_pct, max_loss_pct) = match config.strategy {
        OptionOverlayStrategy::CoveredCall => (
            Some((short_call_rel + short_call_fair - total_friction_pct).round_dp(2)),
            Some((Decimal::from(-100) + short_call_fair - total_friction_pct).round_dp(2)),
        ),
        OptionOverlayStrategy::BullCallSpread => {
            let w = (short_call_rel - atm_rel).max(Decimal::new(5, 1));
            let debit = (atm_call_fair - short_call_fair).max(Decimal::new(10, 2));
            (
                Some((w - debit - total_friction_pct).round_dp(2)),
                Some((-debit - total_friction_pct).round_dp(2)),
            )
        }
        OptionOverlayStrategy::BearPutSpread => {
            let w = (atm_rel - short_put_rel).max(Decimal::new(5, 1));
            let debit = (atm_put_fair - short_put_fair).max(Decimal::new(10, 2));
            (
                Some((w - debit - total_friction_pct).round_dp(2)),
                Some((-debit - total_friction_pct).round_dp(2)),
            )
        }
        OptionOverlayStrategy::IronCondor => {
            let w = (long_call_wing_rel - short_call_rel)
                .max(short_put_rel - long_put_wing_rel)
                .max(Decimal::new(5, 1));
            (
                Some(net_after_friction_pct.round_dp(2)),
                Some((net_after_friction_pct - w).round_dp(2)),
            )
        }
        OptionOverlayStrategy::CalendarSpread => (
            Some((atm_call_fair - total_friction_pct).round_dp(2)),
            Some((net_after_friction_pct - atm_call_fair).round_dp(2)),
        ),
    };

    let pop_pct = if periods.is_empty() {
        Decimal::ZERO
    } else {
        (Decimal::from(win_periods) / Decimal::from(periods.len()) * hundred).round_dp(2)
    };

    // Derive quoted IV proxy from live ATM option quotes when available (Brenner-Subrahmanyam approx).
    let quoted_iv_proxy_pct = if live_chain_available && spot > Decimal::ZERO {
        find_live_quote("CE", atm_strike)
            .and_then(|q| {
                let (mid_opt, _, _) = quote_mid_spread(q);
                mid_opt.map(|mid| {
                    let t_years = (avg_days / Decimal::from(365)).max(Decimal::new(1, 3));
                    let denom = Decimal::new(4, 1) * spot * decimal_sqrt(t_years);
                    if denom > Decimal::ZERO {
                        (mid / denom * hundred).round_dp(2)
                    } else {
                        Decimal::ZERO
                    }
                })
            })
            .filter(|v| *v > Decimal::ZERO)
    } else {
        None
    };
    let iv_minus_rv_pct = quoted_iv_proxy_pct.map(|iv| (iv - empirical_rv_pct).round_dp(2));

    // Build Strike Lab table: use real live chain strikes around spot if available, else
    // surface theoretical strike ladder with `bid: None, ask: None` (zero fabricated quotes!).
    let mut strike_lab = Vec::new();
    if live_chain_available {
        let mut sorted_live = active_quotes;
        sorted_live.sort_by(|a, b| {
            a.strike
                .cmp(&b.strike)
                .then(a.option_type.cmp(&b.option_type))
        });
        let atm_idx = sorted_live
            .iter()
            .position(|q| q.strike >= spot)
            .unwrap_or(sorted_live.len() / 2);
        let lo = atm_idx.saturating_sub(12);
        let hi = (atm_idx + 12).min(sorted_live.len());
        for q in &sorted_live[lo..hi] {
            let (mid, spread_abs, spread_bps) = quote_mid_spread(q);
            let rel_pct = if spot > Decimal::ZERO {
                ((q.strike - spot) / spot * hundred).round_dp(2)
            } else {
                Decimal::ZERO
            };
            let is_call = q.option_type.eq_ignore_ascii_case("CE");
            strike_lab.push(StrikeLabRow {
                option_type: q.option_type.to_ascii_uppercase(),
                strike: q.strike,
                expiry: Some(q.expiry),
                bid: q.bid,
                ask: q.ask,
                mid,
                spread_abs,
                spread_bps,
                liquidity_tier: liquidity_tier_for_bps(spread_bps).to_string(),
                open_interest: q.open_interest,
                exceedance_prob_pct: empirical_exceedance_prob(&sorted, rel_pct, is_call),
                moneyness_pct: rel_pct,
            });
        }
    } else {
        // Unquoted empirical strike ladder anchored to spot + strike_step (bid/ask remain None)
        for offset in -4..=4_i32 {
            let k = (atm_strike + strike_step * Decimal::from(offset)).max(tick);
            let rel_pct = if spot > Decimal::ZERO {
                ((k - spot) / spot * hundred).round_dp(2)
            } else {
                Decimal::ZERO
            };
            let opt_type = if offset < 0 { "PE" } else { "CE" };
            let is_call = opt_type == "CE";
            strike_lab.push(StrikeLabRow {
                option_type: opt_type.to_string(),
                strike: k,
                expiry: None,
                bid: None,
                ask: None,
                mid: None,
                spread_abs: None,
                spread_bps: None,
                liquidity_tier: "unquoted".to_string(),
                open_interest: None,
                exceedance_prob_pct: empirical_exceedance_prob(&sorted, rel_pct, is_call),
                moneyness_pct: rel_pct,
            });
        }
    }

    let availability_note = if live_chain_available {
        format!(
            "Live option chain matched ({} quoted contracts on {}); leg bid/ask spreads and IV-vs-RV use live top-of-book quotes.",
            quoted_count,
            nearest_expiry
                .map(|d| d.to_string())
                .unwrap_or_else(|| "nearest expiry".to_string())
        )
    } else {
        "Live option chain quotes are not loaded for this underlying in the runtime quote store. Empirical payoffs and exceedance probabilities are computed from historical periods with user-configured friction; synthetic bid/ask quotes are never fabricated.".to_string()
    };

    OptionsOverlayReport {
        strategy: config.strategy,
        spot_price: spot.round_dp(2),
        empirical_rv_pct,
        quoted_iv_proxy_pct,
        iv_minus_rv_pct,
        live_chain_available,
        live_contract_count: quoted_count,
        nearest_expiry,
        legs,
        net_premium_pct: net_premium_pct.round_dp(2),
        total_spread_drag_pct: spread_drag_pct.round_dp(2),
        statutory_cost_pct: statutory_cost_pct.round_dp(2),
        net_after_friction_pct: net_after_friction_pct.round_dp(2),
        max_profit_pct,
        max_loss_pct,
        probability_of_profit_pct: pop_pct,
        buy_hold_cum_pct: bh_series.last().copied().unwrap_or(Decimal::ZERO),
        overlay_gross_cum_pct: equity_curve
            .last()
            .map(|p| p.overlay_gross_cum_pct)
            .unwrap_or(Decimal::ZERO),
        overlay_net_cum_pct: net_series.last().copied().unwrap_or(Decimal::ZERO),
        overlay_max_drawdown_pct: compute_max_drawdown_pct(&net_series),
        buy_hold_max_drawdown_pct: compute_max_drawdown_pct(&bh_series),
        win_periods,
        total_periods: periods.len(),
        equity_curve,
        strike_lab,
        availability_note,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Part B: Markov + Volatility Analog Forecasting (`evaluate_forecasting`)
// ─────────────────────────────────────────────────────────────────────────────

/// Trailing volatility regime classification (empirical terciles).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolatilityRegime {
    /// Lower third of trailing volatility history (`<= P33`).
    Low,
    /// Middle third of trailing volatility history (`P33..P67`).
    Normal,
    /// Upper third of trailing volatility history (`>= P67`).
    Elevated,
}

impl VolatilityRegime {
    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low Vol",
            Self::Normal => "Normal Vol",
            Self::Elevated => "Elevated Vol",
        }
    }
}

/// Forward quantile step for Tab 11 (`Forecasting`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastHorizonStep {
    /// Forward step index (`1..=horizon_steps`).
    pub step: usize,
    /// Matched analog sample count reaching `i + step`.
    pub sample_count: usize,
    /// 10th percentile compounded forward return (%).
    pub p10_pct: Decimal,
    /// 25th percentile compounded forward return (%).
    pub p25_pct: Decimal,
    /// 50th percentile (median) compounded forward return (%).
    pub p50_pct: Decimal,
    /// 75th percentile compounded forward return (%).
    pub p75_pct: Decimal,
    /// 90th percentile compounded forward return (%).
    pub p90_pct: Decimal,
    /// Arithmetic mean compounded forward return (%).
    pub mean_pct: Decimal,
    /// Share (%) of analog forward paths finishing positive (`> 0`).
    pub positive_prob_pct: Decimal,
    /// Spot-anchored `P10` target price.
    pub p10_price: Decimal,
    /// Spot-anchored `P50` target price.
    pub p50_price: Decimal,
    /// Spot-anchored `P90` target price.
    pub p90_price: Decimal,
}

/// Historical analog episode matching active Markov + Volatility state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalogEpisode {
    /// Period end date of historical match.
    pub trigger_end_date: NaiveDate,
    /// Period return at match trigger (%).
    pub trigger_return_pct: Decimal,
    /// Markov streak state at match date.
    pub markov_state: MarkovState,
    /// Trailing volatility regime at match date.
    pub vol_regime: VolatilityRegime,
    /// Trailing realized std dev (%) at match date.
    pub trailing_vol_pct: Decimal,
    /// Next-1-period forward return (%).
    pub next_1_pct: Decimal,
    /// Compounded return through full requested horizon (%).
    pub horizon_end_pct: Decimal,
}

/// Complete Markov + Vol Analog Forecast response bundle (Tab 11).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastReport {
    /// Reference current spot price.
    pub spot_price: Decimal,
    /// Current Markov streak state (`None` if zero non-neutral periods).
    pub current_markov_state: Option<MarkovState>,
    /// Current Markov label (`"Short Pos (1-2)"`, etc.).
    pub current_markov_label: String,
    /// Current trailing volatility regime.
    pub current_vol_regime: VolatilityRegime,
    /// Trailing 5-period realized std dev (%).
    pub current_trailing_vol_pct: Decimal,
    /// Relaxation tier used (`"exact_state_and_vol"`, `"markov_state_only"`, `"direction_only"`, or `"unconditional"`).
    pub match_tier: String,
    /// Number of historical analogs selected.
    pub analog_count: usize,
    /// Multi-step quantile fan (`1..=horizon_steps`).
    pub horizon_fan: Vec<ForecastHorizonStep>,
    /// Most recent / closest matching historical episodes (up to 15).
    pub top_analogs: Vec<AnalogEpisode>,
}

fn markov_state_series(periods: &[SlicedPeriod]) -> Vec<Option<MarkovState>> {
    let mut out = Vec::with_capacity(periods.len());
    let mut cur_dir: Option<StreakDirection> = None;
    let mut cur_len = 0usize;
    for p in periods {
        if p.return_pct > Decimal::ZERO {
            if cur_dir == Some(StreakDirection::Positive) {
                cur_len += 1;
            } else {
                cur_dir = Some(StreakDirection::Positive);
                cur_len = 1;
            }
            out.push(Some(MarkovState::from_streak(
                StreakDirection::Positive,
                cur_len,
            )));
        } else if p.return_pct < Decimal::ZERO {
            if cur_dir == Some(StreakDirection::Negative) {
                cur_len += 1;
            } else {
                cur_dir = Some(StreakDirection::Negative);
                cur_len = 1;
            }
            out.push(Some(MarkovState::from_streak(
                StreakDirection::Negative,
                cur_len,
            )));
        } else {
            out.push(cur_dir.map(|d| MarkovState::from_streak(d, cur_len)));
        }
    }
    out
}

/// Evaluates historical analog paths conditioned on Markov state + trailing volatility regime.
pub fn evaluate_forecasting(
    periods: &[SlicedPeriod],
    spot_override: Option<Decimal>,
    horizon_steps: usize,
) -> ForecastReport {
    let horizon = horizon_steps.clamp(1, 12);
    let hundred = Decimal::from(100);
    let spot = spot_override
        .filter(|p| *p > Decimal::ZERO)
        .or_else(|| periods.last().map(|p| p.exit_price))
        .unwrap_or(Decimal::from(100));

    if periods.is_empty() {
        return ForecastReport {
            spot_price: spot.round_dp(2),
            current_markov_state: None,
            current_markov_label: "Neutral".to_string(),
            current_vol_regime: VolatilityRegime::Normal,
            current_trailing_vol_pct: Decimal::ZERO,
            match_tier: "unconditional".to_string(),
            analog_count: 0,
            horizon_fan: Vec::new(),
            top_analogs: Vec::new(),
        };
    }

    let states = markov_state_series(periods);
    let vol_lookback = 5usize;
    let mut trailing_vols = Vec::with_capacity(periods.len());
    for i in 0..periods.len() {
        let lo = i.saturating_sub(vol_lookback - 1);
        let window: Vec<Decimal> = periods[lo..=i].iter().map(|p| p.return_pct).collect();
        let (_, std) = mean_and_sample_std(&window);
        trailing_vols.push(std.round_dp(4));
    }

    let mut sorted_vols = trailing_vols.clone();
    sorted_vols.sort();
    let v33 = empirical_quantile(&sorted_vols, Decimal::from(33));
    let v67 = empirical_quantile(&sorted_vols, Decimal::from(67));
    let classify_vol = |v: Decimal| -> VolatilityRegime {
        if v <= v33 {
            VolatilityRegime::Low
        } else if v >= v67 {
            VolatilityRegime::Elevated
        } else {
            VolatilityRegime::Normal
        }
    };

    let regimes: Vec<VolatilityRegime> = trailing_vols.iter().copied().map(classify_vol).collect();
    let current_state = *states.last().unwrap_or(&None);
    let current_vol = *trailing_vols.last().unwrap_or(&Decimal::ZERO);
    let current_vol_regime = *regimes.last().unwrap_or(&VolatilityRegime::Normal);

    let eligible_end = periods.len().saturating_sub(1);
    let exact: Vec<usize> = (0..eligible_end)
        .filter(|&i| states[i] == current_state && regimes[i] == current_vol_regime)
        .collect();
    let state_only: Vec<usize> = (0..eligible_end)
        .filter(|&i| states[i] == current_state)
        .collect();
    let dir_only: Vec<usize> = (0..eligible_end)
        .filter(|&i| match (states[i], current_state) {
            (Some(a), Some(b)) => a.is_positive() == b.is_positive(),
            _ => false,
        })
        .collect();
    let all_idx: Vec<usize> = (0..eligible_end).collect();

    let (selected, match_tier) = if exact.len() >= 5 {
        (exact, "exact_state_and_vol")
    } else if state_only.len() >= 5 {
        (state_only, "markov_state_only")
    } else if dir_only.len() >= 5 {
        (dir_only, "direction_only")
    } else {
        (all_idx, "unconditional")
    };

    let compounded_after = |start_idx: usize, steps: usize| -> Option<Decimal> {
        if start_idx + steps >= periods.len() {
            return None;
        }
        let mut factor = Decimal::ONE;
        for j in 1..=steps {
            factor *= (Decimal::ONE + periods[start_idx + j].return_pct / hundred)
                .max(Decimal::new(1, 4));
        }
        Some(((factor - Decimal::ONE) * hundred).round_dp(4))
    };

    let mut horizon_fan = Vec::with_capacity(horizon);
    for step in 1..=horizon {
        let mut vals: Vec<Decimal> = selected
            .iter()
            .filter_map(|&idx| compounded_after(idx, step))
            .collect();
        if vals.is_empty() {
            continue;
        }
        vals.sort();
        let (mean_pct, _) = mean_and_sample_std(&vals);
        let p10 = empirical_quantile(&vals, Decimal::from(10)).round_dp(2);
        let p25 = empirical_quantile(&vals, Decimal::from(25)).round_dp(2);
        let p50 = empirical_quantile(&vals, Decimal::from(50)).round_dp(2);
        let p75 = empirical_quantile(&vals, Decimal::from(75)).round_dp(2);
        let p90 = empirical_quantile(&vals, Decimal::from(90)).round_dp(2);
        let pos_count = vals.iter().filter(|v| **v > Decimal::ZERO).count();
        let pos_prob = (Decimal::from(pos_count) / Decimal::from(vals.len()) * hundred).round_dp(2);

        horizon_fan.push(ForecastHorizonStep {
            step,
            sample_count: vals.len(),
            p10_pct: p10,
            p25_pct: p25,
            p50_pct: p50,
            p75_pct: p75,
            p90_pct: p90,
            mean_pct: mean_pct.round_dp(2),
            positive_prob_pct: pos_prob,
            p10_price: (spot * (Decimal::ONE + p10 / hundred)).round_dp(2),
            p50_price: (spot * (Decimal::ONE + p50 / hundred)).round_dp(2),
            p90_price: (spot * (Decimal::ONE + p90 / hundred)).round_dp(2),
        });
    }

    // Top 15 analogs ordered by volatility closeness + recency.
    let mut ranked = selected.clone();
    ranked.sort_by(|a, b| {
        let da = (trailing_vols[*a] - current_vol).abs();
        let db = (trailing_vols[*b] - current_vol).abs();
        da.cmp(&db).then(b.cmp(a))
    });
    let top_analogs: Vec<AnalogEpisode> = ranked
        .into_iter()
        .take(15)
        .filter_map(|idx| {
            let next_1 = compounded_after(idx, 1)?;
            let eff_horizon = horizon.min(periods.len().saturating_sub(1 + idx)).max(1);
            let h_end = compounded_after(idx, eff_horizon)?;
            Some(AnalogEpisode {
                trigger_end_date: periods[idx].end_date,
                trigger_return_pct: periods[idx].return_pct.round_dp(2),
                markov_state: states[idx].unwrap_or(MarkovState::ShortPositive),
                vol_regime: regimes[idx],
                trailing_vol_pct: trailing_vols[idx].round_dp(2),
                next_1_pct: next_1.round_dp(2),
                horizon_end_pct: h_end.round_dp(2),
            })
        })
        .collect();

    ForecastReport {
        spot_price: spot.round_dp(2),
        current_markov_state: current_state,
        current_markov_label: current_state
            .map(|s| s.label().to_string())
            .unwrap_or_else(|| "Neutral".to_string()),
        current_vol_regime,
        current_trailing_vol_pct: current_vol.round_dp(2),
        match_tier: match_tier.to_string(),
        analog_count: selected.len(),
        horizon_fan,
        top_analogs,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Part C: Event Study CAR (`[-20d, +20d]`) Engine (`evaluate_event_study`)
// ─────────────────────────────────────────────────────────────────────────────

/// Event study category aligned across Phase 3 Event Modes & Tab 12.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyEventKind {
    /// Share buyback / tender / bonus / corporate action.
    Buyback,
    /// Quarterly earnings or financial result release.
    Earnings,
    /// Sovereign / central bank macro release (RBI / FOMC / CPI / GDP).
    Macro,
    /// F&O inclusion, exclusion, or index rebalance corporate event.
    FnoRebalance,
}

/// One dated event record passed to the CAR engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudyEventInput {
    /// Calendar date of event announcement or release.
    pub event_date: NaiveDate,
    /// Symbol / instrument label.
    pub symbol: String,
    /// Event classification.
    pub kind: StudyEventKind,
    /// Human-readable title (`"Q3 Results"`, `"RBI Repo Rate"`, etc.).
    pub title: String,
    /// Optional numeric or category detail (e.g. `"Actual 6.50% vs Forecast 6.50%"`).
    pub detail: Option<String>,
}

/// Average & Cumulative Abnormal Return sample for one offset `k ∈ [-20..=+20]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventWindowPoint {
    /// Trading-day offset relative to event `T+0` (`-20..=+20`).
    pub day_offset: i32,
    /// Number of events with valid trading bars at this offset.
    pub sample_count: usize,
    /// Cross-event average abnormal daily return (%).
    pub avg_abnormal_return_pct: Decimal,
    /// Cumulative average abnormal return (%) from `T-20` through `day_offset`.
    pub cum_abnormal_return_pct: Decimal,
    /// Cross-event median abnormal return (%) at `day_offset`.
    pub median_return_pct: Decimal,
    /// Percentage of positive abnormal returns at `day_offset`.
    pub win_rate_pct: Decimal,
}

/// Per-event drift summary row for event study tables.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventStudyRow {
    /// Aligned trading date (`T+0`) on or immediately following `event_date`.
    pub event_date: NaiveDate,
    /// Instrument symbol.
    pub symbol: String,
    /// Event kind.
    pub kind: StudyEventKind,
    /// Title.
    pub title: String,
    /// Optional detail string.
    pub detail: Option<String>,
    /// Pre-event drift (`T-20` to `T-1`, %).
    pub pre_drift_pct: Decimal,
    /// Event-day abnormal return (`T+0`, %).
    pub event_day_pct: Decimal,
    /// Post-event 5-session drift (`T+1` to `T+5`, %).
    pub post_5d_pct: Decimal,
    /// Post-event 20-session drift (`T+1` to `T+20`, %).
    pub post_20d_pct: Decimal,
    /// Event-day volume relative to trailing 20-day pre-event average (`×`).
    pub volume_ratio: Decimal,
}

/// Complete `[-20d, +20d]` Event Study report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventStudyReport {
    /// `true` when at least one event aligned with daily bars.
    pub available: bool,
    /// `true` when daily returns were benchmark-adjusted against `benchmark_bars`.
    pub benchmark_adjusted: bool,
    /// Benchmark label if adjusted.
    pub benchmark_symbol: Option<String>,
    /// Count of aligned events.
    pub event_count: usize,
    /// Cross-event mean pre-event drift (`[-20d, -1d]`, %).
    pub avg_pre_drift_pct: Decimal,
    /// Cross-event mean announcement return (`T+0`, %).
    pub avg_event_day_pct: Decimal,
    /// Cross-event mean `+5d` drift (`[+1d, +5d]`, %).
    pub avg_post_5d_pct: Decimal,
    /// Cross-event mean `+20d` drift (`[+1d, +20d]`, %).
    pub avg_post_20d_pct: Decimal,
    /// Win rate (%) for `T+1` cumulative abnormal drift (`[+1d]`).
    pub win_rate_post_1d_pct: Decimal,
    /// Win rate (%) for `T+5` cumulative abnormal drift (`[+1d..+5d]`).
    pub win_rate_post_5d_pct: Decimal,
    /// Win rate (%) for `T+20` cumulative abnormal drift (`[+1d..+20d]`).
    pub win_rate_post_20d_pct: Decimal,
    /// Average event-day volume ratio vs pre-event 20-day mean.
    pub avg_volume_surge_ratio: Decimal,
    /// 41-point `[-20..=+20]` AAR / CAR trajectory.
    pub window_curve: Vec<EventWindowPoint>,
    /// Most recent aligned event rows (up to 100).
    pub events: Vec<EventStudyRow>,
    /// Honest availability/data-source explanation.
    pub availability_note: String,
}

/// Evaluates `[-20d, +20d]` benchmark-adjusted CAR & volume reaction over daily OHLCV bars.
pub fn evaluate_event_study(
    bars: &[OhlcvBar],
    benchmark_bars: Option<&[OhlcvBar]>,
    benchmark_symbol: Option<&str>,
    events: &[StudyEventInput],
) -> EventStudyReport {
    let hundred = Decimal::from(100);
    if bars.len() < 2 || events.is_empty() {
        return EventStudyReport {
            available: false,
            benchmark_adjusted: false,
            benchmark_symbol: benchmark_symbol.map(str::to_string),
            event_count: 0,
            avg_pre_drift_pct: Decimal::ZERO,
            avg_event_day_pct: Decimal::ZERO,
            avg_post_5d_pct: Decimal::ZERO,
            avg_post_20d_pct: Decimal::ZERO,
            win_rate_post_1d_pct: Decimal::ZERO,
            win_rate_post_5d_pct: Decimal::ZERO,
            win_rate_post_20d_pct: Decimal::ZERO,
            avg_volume_surge_ratio: Decimal::ONE,
            window_curve: Vec::new(),
            events: Vec::new(),
            availability_note: if events.is_empty() {
                "No historical event records (corporate actions, filings, or economic releases) matched in Lakehouse projections for this instrument/date range. Consistent with the zero-fabrication policy, synthetic events are never generated.".to_string()
            } else {
                "Insufficient daily OHLCV bars to align event windows.".to_string()
            },
        };
    }

    // Build benchmark daily return map by date if supplied.
    let mut bench_ret: HashMap<NaiveDate, Decimal> = HashMap::new();
    if let Some(bb) = benchmark_bars {
        for w in bb.windows(2) {
            if w[0].close > Decimal::ZERO {
                let r = ((w[1].close - w[0].close) / w[0].close * hundred).round_dp(4);
                bench_ret.insert(w[1].date, r);
            }
        }
    }
    let benchmark_adjusted = !bench_ret.is_empty();

    // Daily abnormal returns indexed alongside `bars`.
    let mut daily_ar = vec![Decimal::ZERO; bars.len()];
    for i in 1..bars.len() {
        let raw_r = if bars[i - 1].close > Decimal::ZERO {
            ((bars[i].close - bars[i - 1].close) / bars[i - 1].close * hundred).round_dp(4)
        } else {
            Decimal::ZERO
        };
        let b_r = bench_ret
            .get(&bars[i].date)
            .copied()
            .unwrap_or(Decimal::ZERO);
        daily_ar[i] = (raw_r - b_r).round_dp(4);
    }

    // Bucket abnormal returns by integer offset [-20..=+20].
    let mut by_offset: BTreeMap<i32, Vec<Decimal>> = BTreeMap::new();
    for k in -20..=20 {
        by_offset.insert(k, Vec::new());
    }

    let mut rows = Vec::new();
    let mut post_1d_vals = Vec::new();
    let mut post_5d_vals = Vec::new();
    let mut post_20d_vals = Vec::new();
    let mut pre_vals = Vec::new();
    let mut t0_vals = Vec::new();
    let mut vol_ratios = Vec::new();

    for ev in events {
        // Align to exact trading day or first trading session after weekend/holiday (within 5 cal days).
        let pos = bars.partition_point(|b| b.date < ev.event_date);
        if pos >= bars.len() || (bars[pos].date - ev.event_date).num_days() > 5 {
            continue;
        }
        let anchor_date = bars[pos].date;

        for k in -20..=20_i32 {
            let target = pos as isize + k as isize;
            if target >= 1
                && (target as usize) < bars.len()
                && let Some(bucket) = by_offset.get_mut(&k)
            {
                bucket.push(daily_ar[target as usize]);
            }
        }

        let sum_range = |lo: isize, hi: isize| -> Decimal {
            let mut acc = Decimal::ZERO;
            for off in lo..=hi {
                let idx = pos as isize + off;
                if idx >= 1 && (idx as usize) < bars.len() {
                    acc += daily_ar[idx as usize];
                }
            }
            acc.round_dp(2)
        };

        let pre_drift = sum_range(-20, -1);
        let event_day = daily_ar[pos].round_dp(2);
        let post_1d = sum_range(1, 1);
        let post_5d = sum_range(1, 5);
        let post_20d = sum_range(1, 20);

        let mut pre_vols = Vec::new();
        for off in -20..=-1_isize {
            let idx = pos as isize + off;
            if idx >= 0 && (idx as usize) < bars.len() && bars[idx as usize].volume > Decimal::ZERO
            {
                pre_vols.push(bars[idx as usize].volume);
            }
        }
        let vol_ratio = if !pre_vols.is_empty() && bars[pos].volume > Decimal::ZERO {
            let (avg_v, _) = mean_and_sample_std(&pre_vols);
            if avg_v > Decimal::ZERO {
                (bars[pos].volume / avg_v).round_dp(2)
            } else {
                Decimal::ONE
            }
        } else {
            Decimal::ONE
        };

        pre_vals.push(pre_drift);
        t0_vals.push(event_day);
        post_1d_vals.push(post_1d);
        post_5d_vals.push(post_5d);
        post_20d_vals.push(post_20d);
        vol_ratios.push(vol_ratio);

        rows.push(EventStudyRow {
            event_date: anchor_date,
            symbol: ev.symbol.clone(),
            kind: ev.kind,
            title: ev.title.clone(),
            detail: ev.detail.clone(),
            pre_drift_pct: pre_drift,
            event_day_pct: event_day,
            post_5d_pct: post_5d,
            post_20d_pct: post_20d,
            volume_ratio: vol_ratio,
        });
    }

    if rows.is_empty() {
        return EventStudyReport {
            available: false,
            benchmark_adjusted,
            benchmark_symbol: benchmark_symbol.map(str::to_string),
            event_count: 0,
            avg_pre_drift_pct: Decimal::ZERO,
            avg_event_day_pct: Decimal::ZERO,
            avg_post_5d_pct: Decimal::ZERO,
            avg_post_20d_pct: Decimal::ZERO,
            win_rate_post_1d_pct: Decimal::ZERO,
            win_rate_post_5d_pct: Decimal::ZERO,
            win_rate_post_20d_pct: Decimal::ZERO,
            avg_volume_surge_ratio: Decimal::ONE,
            window_curve: Vec::new(),
            events: Vec::new(),
            availability_note:
                "Event dates fell outside the available daily OHLCV bar range for this instrument."
                    .to_string(),
        };
    }

    let mut cum_car = Decimal::ZERO;
    let mut window_curve = Vec::with_capacity(41);
    for (k, mut vals) in by_offset {
        if vals.is_empty() {
            window_curve.push(EventWindowPoint {
                day_offset: k,
                sample_count: 0,
                avg_abnormal_return_pct: Decimal::ZERO,
                cum_abnormal_return_pct: cum_car.round_dp(3),
                median_return_pct: Decimal::ZERO,
                win_rate_pct: Decimal::ZERO,
            });
            continue;
        }
        vals.sort();
        let (aar, _) = mean_and_sample_std(&vals);
        let med = empirical_quantile(&vals, Decimal::from(50)).round_dp(3);
        let wins = vals.iter().filter(|v| **v > Decimal::ZERO).count();
        let win_pct = (Decimal::from(wins) / Decimal::from(vals.len()) * hundred).round_dp(2);
        cum_car += aar;
        window_curve.push(EventWindowPoint {
            day_offset: k,
            sample_count: vals.len(),
            avg_abnormal_return_pct: aar.round_dp(3),
            cum_abnormal_return_pct: cum_car.round_dp(3),
            median_return_pct: med,
            win_rate_pct: win_pct,
        });
    }

    let win_rate = |vals: &[Decimal]| -> Decimal {
        if vals.is_empty() {
            return Decimal::ZERO;
        }
        let w = vals.iter().filter(|v| **v > Decimal::ZERO).count();
        (Decimal::from(w) / Decimal::from(vals.len()) * hundred).round_dp(2)
    };

    let (avg_pre, _) = mean_and_sample_std(&pre_vals);
    let (avg_t0, _) = mean_and_sample_std(&t0_vals);
    let (avg_p5, _) = mean_and_sample_std(&post_5d_vals);
    let (avg_p20, _) = mean_and_sample_std(&post_20d_vals);
    let (avg_vol, _) = mean_and_sample_std(&vol_ratios);

    rows.sort_by_key(|b| std::cmp::Reverse(b.event_date));
    rows.truncate(100);

    let event_count = post_5d_vals.len();
    let availability_note = format!(
        "Evaluated {} historical events over `[-20d, +20d]` windows{}.",
        event_count,
        if benchmark_adjusted {
            format!(
                " (benchmark-adjusted vs {})",
                benchmark_symbol.unwrap_or("benchmark")
            )
        } else {
            " (raw excess returns; overlapping benchmark bars unavailable)".to_string()
        }
    );

    EventStudyReport {
        available: true,
        benchmark_adjusted,
        benchmark_symbol: benchmark_symbol.map(str::to_string),
        event_count,
        avg_pre_drift_pct: avg_pre.round_dp(2),
        avg_event_day_pct: avg_t0.round_dp(2),
        avg_post_5d_pct: avg_p5.round_dp(2),
        avg_post_20d_pct: avg_p20.round_dp(2),
        win_rate_post_1d_pct: win_rate(&post_1d_vals),
        win_rate_post_5d_pct: win_rate(&post_5d_vals),
        win_rate_post_20d_pct: win_rate(&post_20d_vals),
        avg_volume_surge_ratio: avg_vol.round_dp(2),
        window_curve,
        events: rows,
        availability_note,
    }
}
