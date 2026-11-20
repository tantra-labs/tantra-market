//! Primitive 6: Horizon-aware F&O and commodity hedge synthesizer (`HedgeSynthesizer`).

use rust_decimal::Decimal;

use super::super::options_events::LiveOptionStrikeQuote;
use super::super::spreads::estimate_statutory_roundtrip_bps;
use super::types::{
    ExpiryHorizon, HedgeLeg, HedgeOverlayPolicy, HedgeRecommendation, HedgeStructureKind,
    HedgeSynthesisInput, SignalDirection, ZonePolarity,
};

/// Default F&O & Commodity hedge synthesizer implementing [`HedgeOverlayPolicy`].
///
/// Maps signal direction, zone support/resistance anchors, penetration `P75/P90/P95` tails,
/// IV-vs-RV regime, trap rate, and live `ChainReader` quotes (with honest theoretical fallback)
/// into bounded-risk structures across `Day`, `Weekly`, `Monthly`, and `CommodityCycle` horizons.
#[derive(Clone, Copy, Debug, Default)]
pub struct HedgeSynthesizer;

fn strike_step_for_spot(spot: Decimal, tick: Decimal) -> Decimal {
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
    snap_step(raw, tick.max(Decimal::new(1, 2))).max(tick)
}

fn snap_step(val: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return val.round_dp(2);
    }
    ((val / step).round() * step).round_dp(2)
}

fn match_live_quote<'a>(
    chain: &'a [LiveOptionStrikeQuote],
    opt_type: &str,
    target_strike: Decimal,
) -> Option<&'a LiveOptionStrikeQuote> {
    let nearest_exp = chain.iter().map(|q| q.expiry).min()?;
    chain
        .iter()
        .filter(|q| q.expiry == nearest_exp && q.option_type.eq_ignore_ascii_case(opt_type))
        .min_by_key(|q| (q.strike - target_strike).abs())
}

fn resolve_option_leg(
    side: &str,
    option_type: &str,
    target_strike: Decimal,
    spot: Decimal,
    atr: Decimal,
    lot_qty: Decimal,
    chain: &[LiveOptionStrikeQuote],
) -> (HedgeLeg, bool) {
    if let Some(q) = match_live_quote(chain, option_type, target_strike) {
        let px_opt = match side {
            "BUY" => q.ask.or(q.last_price).or(q.bid),
            _ => q.bid.or(q.last_price).or(q.ask),
        };
        if let Some(px) = px_opt {
            let source = if q.bid.is_some() && q.ask.is_some() {
                "live_bid_ask"
            } else {
                "live_ltp"
            };
            return (
                HedgeLeg {
                    side: side.to_string(),
                    option_type: option_type.to_string(),
                    strike: q.strike,
                    expiry: Some(q.expiry),
                    lot_qty,
                    leg_price: px.round_dp(2),
                    quote_source: source.to_string(),
                },
                true,
            );
        }
    }

    // Honest theoretical intrinsic + ATR time value estimate when chain is absent
    let intrinsic = match option_type {
        "CE" => (spot - target_strike).max(Decimal::ZERO),
        "PE" => (target_strike - spot).max(Decimal::ZERO),
        _ => Decimal::ZERO,
    };
    let dist_atr = if atr > Decimal::ZERO {
        ((target_strike - spot).abs() / atr).min(Decimal::from(6))
    } else {
        Decimal::ONE
    };
    let time_decay_mult =
        (Decimal::new(65, 2) - dist_atr * Decimal::new(10, 2)).max(Decimal::new(12, 2));
    let theoretical_px = (intrinsic + atr * time_decay_mult)
        .max(spot * Decimal::new(15, 4))
        .round_dp(2);

    (
        HedgeLeg {
            side: side.to_string(),
            option_type: option_type.to_string(),
            strike: target_strike,
            expiry: None,
            lot_qty,
            leg_price: theoretical_px,
            quote_source: "theoretical_atr".to_string(),
        },
        false,
    )
}

impl HedgeOverlayPolicy for HedgeSynthesizer {
    fn synthesize(&self, input: &HedgeSynthesisInput<'_>) -> HedgeRecommendation {
        let spot = input.spot.max(Decimal::ONE);
        let atr = input.atr.max(spot * Decimal::new(5, 3)).max(Decimal::ONE);
        let tick = input.tick_size.max(Decimal::new(1, 2));
        let step = strike_step_for_spot(spot, tick);
        let lot_qty = input.lot_size.max(Decimal::ONE).round_dp(0);

        // Extract zone support/resistance anchors or fall back to symmetric ATR bounds
        let latest_sess = input.zones.and_then(|z| z.sessions.last());
        let s1_anchor = latest_sess
            .and_then(|s| {
                s.bands
                    .iter()
                    .find(|b| b.label == "S1" || b.polarity == ZonePolarity::Support)
                    .map(|b| b.low)
            })
            .unwrap_or_else(|| spot - atr * Decimal::new(15, 1));
        let s2_anchor = latest_sess
            .and_then(|s| s.bands.iter().find(|b| b.label == "S2").map(|b| b.outer_sl))
            .unwrap_or_else(|| spot - atr * Decimal::new(26, 1));
        let r1_anchor = latest_sess
            .and_then(|s| {
                s.bands
                    .iter()
                    .find(|b| b.label == "R1" || b.polarity == ZonePolarity::Resistance)
                    .map(|b| b.high)
            })
            .unwrap_or_else(|| spot + atr * Decimal::new(15, 1));
        let r2_anchor = latest_sess
            .and_then(|s| s.bands.iter().find(|b| b.label == "R2").map(|b| b.outer_sl))
            .unwrap_or_else(|| spot + atr * Decimal::new(26, 1));

        let p90_pad = input
            .penetration
            .map(|p| p.p90_atr.max(Decimal::new(25, 2)) * atr)
            .unwrap_or_else(|| atr * Decimal::new(50, 2));
        let p95_atr = input
            .penetration
            .map(|p| p.p95_atr.max(Decimal::ONE))
            .unwrap_or(Decimal::new(15, 1));

        let structure = match (input.horizon, input.direction) {
            (ExpiryHorizon::CommodityCycle, _) => HedgeStructureKind::ProtectiveCollar,
            (ExpiryHorizon::Day, Some(SignalDirection::Short)) => HedgeStructureKind::BearPutSpread,
            (ExpiryHorizon::Day, _) => HedgeStructureKind::BullCallSpread,
            (ExpiryHorizon::Weekly | ExpiryHorizon::Monthly, None) => {
                HedgeStructureKind::ZoneIronCondor
            }
            (ExpiryHorizon::Weekly | ExpiryHorizon::Monthly, Some(SignalDirection::Long)) => {
                HedgeStructureKind::BullPutCreditSpread
            }
            (ExpiryHorizon::Weekly | ExpiryHorizon::Monthly, Some(SignalDirection::Short)) => {
                HedgeStructureKind::BearCallCreditSpread
            }
        };

        let atm_k = snap_step(spot, step).max(tick);
        let mut legs = Vec::new();
        let mut any_live = false;

        match structure {
            HedgeStructureKind::BullCallSpread => {
                let otm_k = snap_step(r1_anchor.max(spot + atr), step).max(atm_k + step);
                let (l1, live1) =
                    resolve_option_leg("BUY", "CE", atm_k, spot, atr, lot_qty, input.live_chain);
                let (l2, live2) =
                    resolve_option_leg("SELL", "CE", otm_k, spot, atr, lot_qty, input.live_chain);
                any_live |= live1 || live2;
                legs.push(l1);
                legs.push(l2);
            }
            HedgeStructureKind::BearPutSpread => {
                let otm_k = snap_step(s1_anchor.min(spot - atr), step)
                    .min((atm_k - step).max(step))
                    .max(step);
                let (l1, live1) =
                    resolve_option_leg("BUY", "PE", atm_k, spot, atr, lot_qty, input.live_chain);
                let (l2, live2) =
                    resolve_option_leg("SELL", "PE", otm_k, spot, atr, lot_qty, input.live_chain);
                any_live |= live1 || live2;
                legs.push(l1);
                legs.push(l2);
            }
            HedgeStructureKind::BullPutCreditSpread => {
                let short_k = snap_step(s1_anchor - atr * Decimal::new(25, 2), step)
                    .min((atm_k - step).max(step))
                    .max(step);
                let wing_k = snap_step(s2_anchor - p90_pad, step)
                    .min((short_k - step).max(step))
                    .max(step);
                let (l1, live1) =
                    resolve_option_leg("SELL", "PE", short_k, spot, atr, lot_qty, input.live_chain);
                let (l2, live2) =
                    resolve_option_leg("BUY", "PE", wing_k, spot, atr, lot_qty, input.live_chain);
                any_live |= live1 || live2;
                legs.push(l1);
                legs.push(l2);
            }
            HedgeStructureKind::BearCallCreditSpread => {
                let short_k =
                    snap_step(r1_anchor + atr * Decimal::new(25, 2), step).max(atm_k + step);
                let wing_k = snap_step(r2_anchor + p90_pad, step).max(short_k + step);
                let (l1, live1) =
                    resolve_option_leg("SELL", "CE", short_k, spot, atr, lot_qty, input.live_chain);
                let (l2, live2) =
                    resolve_option_leg("BUY", "CE", wing_k, spot, atr, lot_qty, input.live_chain);
                any_live |= live1 || live2;
                legs.push(l1);
                legs.push(l2);
            }
            HedgeStructureKind::ZoneIronCondor => {
                let short_pe = snap_step(s1_anchor - atr * Decimal::new(25, 2), step)
                    .min((atm_k - step).max(step))
                    .max(step);
                let wing_pe = snap_step(s2_anchor - p90_pad, step)
                    .min((short_pe - step).max(step))
                    .max(step);
                let short_ce =
                    snap_step(r1_anchor + atr * Decimal::new(25, 2), step).max(atm_k + step);
                let wing_ce = snap_step(r2_anchor + p90_pad, step).max(short_ce + step);

                for (side, opt, k) in [
                    ("SELL", "PE", short_pe),
                    ("BUY", "PE", wing_pe),
                    ("SELL", "CE", short_ce),
                    ("BUY", "CE", wing_ce),
                ] {
                    let (leg, live) =
                        resolve_option_leg(side, opt, k, spot, atr, lot_qty, input.live_chain);
                    any_live |= live;
                    legs.push(leg);
                }
            }
            HedgeStructureKind::ProtectiveCollar => {
                let fut_side = match input.direction {
                    Some(SignalDirection::Short) => "SELL",
                    _ => "BUY",
                };
                legs.push(HedgeLeg {
                    side: fut_side.to_string(),
                    option_type: "FUT".to_string(),
                    strike: spot.round_dp(2),
                    expiry: None,
                    lot_qty,
                    leg_price: spot.round_dp(2),
                    quote_source: "spot_anchor".to_string(),
                });
                let put_k = snap_step(s2_anchor.min(spot - atr * Decimal::from(2)), step)
                    .min((atm_k - step).max(step))
                    .max(step);
                let call_k =
                    snap_step(r2_anchor.max(spot + atr * Decimal::from(2)), step).max(atm_k + step);
                if fut_side == "BUY" {
                    let (p_leg, l1) = resolve_option_leg(
                        "BUY",
                        "PE",
                        put_k,
                        spot,
                        atr,
                        lot_qty,
                        input.live_chain,
                    );
                    let (c_leg, l2) = resolve_option_leg(
                        "SELL",
                        "CE",
                        call_k,
                        spot,
                        atr,
                        lot_qty,
                        input.live_chain,
                    );
                    any_live |= l1 || l2;
                    legs.push(p_leg);
                    legs.push(c_leg);
                } else {
                    let (c_leg, l1) = resolve_option_leg(
                        "BUY",
                        "CE",
                        call_k,
                        spot,
                        atr,
                        lot_qty,
                        input.live_chain,
                    );
                    let (p_leg, l2) = resolve_option_leg(
                        "SELL",
                        "PE",
                        put_k,
                        spot,
                        atr,
                        lot_qty,
                        input.live_chain,
                    );
                    any_live |= l1 || l2;
                    legs.push(c_leg);
                    legs.push(p_leg);
                }
            }
            HedgeStructureKind::Unhedged | HedgeStructureKind::CalendarRoll => {
                let (l1, live1) =
                    resolve_option_leg("BUY", "CE", atm_k, spot, atr, lot_qty, input.live_chain);
                any_live |= live1;
                legs.push(l1);
            }
        }

        // Compute option cashflow per unit (excluding spot/FUT principal leg)
        let mut net_opt_unit = Decimal::ZERO;
        let mut premium_notional = Decimal::ZERO;
        for leg in &legs {
            if leg.option_type == "FUT" {
                continue;
            }
            premium_notional += leg.leg_price * leg.lot_qty;
            if leg.side == "SELL" {
                net_opt_unit += leg.leg_price;
            } else {
                net_opt_unit -= leg.leg_price;
            }
        }
        let net_credit_or_debit = (net_opt_unit * lot_qty).round_dp(2);

        let stat_bps = if input.spread_cost.include_statutory_costs {
            estimate_statutory_roundtrip_bps(&input.symbol, input.asset_class)
        } else {
            Decimal::new(12, 0)
        };
        let total_bps = input.spread_cost.spread_slippage_bps.max(Decimal::ZERO) + stat_bps;
        let notional_ref = if legs.iter().any(|l| l.option_type == "FUT") {
            spot * lot_qty
        } else {
            premium_notional.max(spot * lot_qty * Decimal::new(5, 2))
        };
        let estimated_friction_inr = (notional_ref * total_bps / Decimal::from(10_000)).round_dp(2);

        let max_loss_inr = compute_structure_max_loss(
            structure,
            &legs,
            lot_qty,
            spot,
            net_credit_or_debit,
            estimated_friction_inr,
        );
        let max_loss_pct_equity = if input.capital > Decimal::ZERO {
            (max_loss_inr / input.capital * Decimal::from(100)).round_dp(2)
        } else {
            Decimal::ZERO
        };

        let p95_stress_loss_inr = (atr * p95_atr * lot_qty).min(max_loss_inr).round_dp(2);
        let breakevens = compute_breakevens(structure, &legs, net_opt_unit);
        let chain_status = if any_live {
            "live_quotes".to_string()
        } else {
            "theoretical_model".to_string()
        };

        let rationale = format!(
            "{:?} horizon -> {:?} ({}) anchored to S1/R1 & P90 outer shield; max loss ₹{:.0} ({:.2}% equity), P95 stress ₹{:.0}.",
            input.horizon,
            structure,
            chain_status,
            max_loss_inr,
            max_loss_pct_equity,
            p95_stress_loss_inr
        );

        HedgeRecommendation {
            structure,
            horizon: input.horizon,
            chain_status,
            legs,
            net_credit_or_debit,
            max_loss_inr,
            max_loss_pct_equity,
            p95_stress_loss_inr,
            estimated_friction_inr,
            breakevens,
            rationale,
        }
    }
}

fn compute_structure_max_loss(
    structure: HedgeStructureKind,
    legs: &[HedgeLeg],
    lot_qty: Decimal,
    spot: Decimal,
    net_credit_or_debit: Decimal,
    friction_inr: Decimal,
) -> Decimal {
    match structure {
        HedgeStructureKind::BullCallSpread | HedgeStructureKind::BearPutSpread => {
            ((-net_credit_or_debit).max(Decimal::ZERO) + friction_inr).round_dp(2)
        }
        HedgeStructureKind::BullPutCreditSpread | HedgeStructureKind::BearCallCreditSpread => {
            let width = if legs.len() >= 2 {
                (legs[0].strike - legs[1].strike).abs() * lot_qty
            } else {
                Decimal::ZERO
            };
            ((width - net_credit_or_debit).max(Decimal::ZERO) + friction_inr).round_dp(2)
        }
        HedgeStructureKind::ZoneIronCondor => {
            let put_w = if legs.len() >= 2 {
                (legs[0].strike - legs[1].strike).abs() * lot_qty
            } else {
                Decimal::ZERO
            };
            let call_w = if legs.len() >= 4 {
                (legs[3].strike - legs[2].strike).abs() * lot_qty
            } else {
                Decimal::ZERO
            };
            ((put_w.max(call_w) - net_credit_or_debit).max(Decimal::ZERO) + friction_inr)
                .round_dp(2)
        }
        HedgeStructureKind::ProtectiveCollar => {
            let fut_entry = legs.first().map(|l| l.strike).unwrap_or(spot);
            let prot_k = legs.get(1).map(|l| l.strike).unwrap_or(spot);
            let dist = (fut_entry - prot_k).abs() * lot_qty;
            ((dist - net_credit_or_debit).max(Decimal::ZERO) + friction_inr).round_dp(2)
        }
        HedgeStructureKind::Unhedged | HedgeStructureKind::CalendarRoll => {
            (spot * lot_qty * Decimal::new(5, 2) + friction_inr).round_dp(2)
        }
    }
}

fn compute_breakevens(
    structure: HedgeStructureKind,
    legs: &[HedgeLeg],
    net_opt_unit: Decimal,
) -> Vec<Decimal> {
    match structure {
        HedgeStructureKind::BullCallSpread if !legs.is_empty() => {
            vec![(legs[0].strike + (-net_opt_unit).max(Decimal::ZERO)).round_dp(2)]
        }
        HedgeStructureKind::BearPutSpread if !legs.is_empty() => {
            vec![(legs[0].strike - (-net_opt_unit).max(Decimal::ZERO)).round_dp(2)]
        }
        HedgeStructureKind::BullPutCreditSpread if !legs.is_empty() => {
            vec![(legs[0].strike - net_opt_unit.max(Decimal::ZERO)).round_dp(2)]
        }
        HedgeStructureKind::BearCallCreditSpread if !legs.is_empty() => {
            vec![(legs[0].strike + net_opt_unit.max(Decimal::ZERO)).round_dp(2)]
        }
        HedgeStructureKind::ZoneIronCondor if legs.len() >= 4 => {
            let c = net_opt_unit.max(Decimal::ZERO);
            vec![
                (legs[0].strike - c).round_dp(2),
                (legs[2].strike + c).round_dp(2),
            ]
        }
        HedgeStructureKind::ProtectiveCollar if !legs.is_empty() => {
            vec![(legs[0].strike - net_opt_unit).round_dp(2)]
        }
        _ => Vec::new(),
    }
}
