//! F&O spread, futures roll-basis & statutory friction modeling (`Decimal`-only).
//!
//! Converts spot-based sliced periods into net-of-spread / net-of-cost /
//! basis-adjusted returns using exact statutory Indian derivative rates from
//! [`crate::india::costs::segments`] paired with user-configured bid-ask spread
//! and futures carry assumptions.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tantra_core::Exchange;

use crate::india::costs::segments::{SegmentProduct, turnover_fee_rate};

use super::returns::{ReturnKpis, compute_return_series_and_kpis};
use super::schedule::{ReturnCalcKind, SlicedPeriod, StudyAssetClass};

/// Return adjustment regime applied over sliced periods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RollBasisMode {
    /// Unadjusted spot / bar returns (Phase 1 default).
    #[default]
    GrossSpot,
    /// Applies futures rollover/carry basis (`futures_roll_basis_bps`) to spot periods.
    IncludeRollBasis,
    /// Applies rollover basis AND deducts bid-ask spread slippage + statutory exchange/STT costs.
    NetAfterSpreadAndCosts,
}

/// Configuration for F&O spread, rollover basis, and statutory friction adjustments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpreadCostConfig {
    /// Active adjustment mode.
    pub mode: RollBasisMode,
    /// Round-trip bid-ask spread slippage in basis points (`0..=500`, e.g. `10` = `0.10%`).
    pub spread_slippage_bps: Decimal,
    /// Futures carry / rollover basis adjustment per period in basis points (`-500..=500`).
    pub futures_roll_basis_bps: Decimal,
    /// Whether to deduct Indian statutory turnover charges (`STT` + `exchange_fee` + `GST` + `stamp` + `SEBI`).
    pub include_statutory_costs: bool,
}

impl Default for SpreadCostConfig {
    fn default() -> Self {
        Self {
            mode: RollBasisMode::GrossSpot,
            spread_slippage_bps: Decimal::ZERO,
            futures_roll_basis_bps: Decimal::ZERO,
            include_statutory_costs: false,
        }
    }
}

/// Gross vs Net comparison summary quantifying F&O spread, basis, and fee drag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrictionSummary {
    /// Active roll/friction adjustment mode.
    pub mode: RollBasisMode,
    /// Gross arithmetic average period return (%) before friction.
    pub gross_avg_return_pct: Decimal,
    /// Net arithmetic average period return (%) after spread, basis, and statutory costs.
    pub net_avg_return_pct: Decimal,
    /// Gross period Sharpe ratio.
    pub gross_sharpe_ratio: Decimal,
    /// Net period Sharpe ratio after friction.
    pub net_sharpe_ratio: Decimal,
    /// Gross profit factor.
    pub gross_profit_factor: Decimal,
    /// Net profit factor after friction.
    pub net_profit_factor: Decimal,
    /// Configured round-trip bid-ask spread slippage per period (bps).
    pub per_period_spread_bps: Decimal,
    /// Round-trip statutory derivative turnover charge per period (bps).
    pub per_period_statutory_bps: Decimal,
    /// Configured futures roll/carry basis adjustment per period (bps).
    pub per_period_basis_bps: Decimal,
    /// Cumulative percentage-point impact across all evaluated periods (`sum(net - gross)`).
    pub cumulative_net_delta_pct: Decimal,
}

/// Parses an instrument/exchange string into a canonical [`Exchange`] for derivative rate lookup.
pub(crate) fn resolve_exchange_for_costs(
    exchange_or_symbol: &str,
    asset_class: StudyAssetClass,
) -> Exchange {
    let upper = exchange_or_symbol.trim().to_ascii_uppercase();
    if upper.starts_with("MCX") || asset_class == StudyAssetClass::Commodity {
        return Exchange::Mcx;
    }
    if upper.starts_with("CDS") || asset_class == StudyAssetClass::Forex {
        return Exchange::Cds;
    }
    if upper.starts_with("NFO")
        || asset_class == StudyAssetClass::FnoStock
        || asset_class == StudyAssetClass::Index
    {
        return Exchange::Nfo;
    }
    Exchange::Nse
}

/// Computes exact round-trip statutory turnover charge in basis points (`Decimal`) for one cycle.
///
/// Reuses the exact `Decimal` segment rates in [`crate::india::costs::segments`] plus current
/// Indian statutory rates (sell-side STT, buy-side stamp duty, SEBI fee, and 18% GST).
pub fn estimate_statutory_roundtrip_bps(
    exchange_or_symbol: &str,
    asset_class: StudyAssetClass,
) -> Decimal {
    let ex = resolve_exchange_for_costs(exchange_or_symbol, asset_class);
    let ten_k = Decimal::from(10_000);
    let gst_mult = Decimal::new(118, 2); // 1 + 18% GST on exchange & SEBI fees
    let sebi_one_side = Decimal::new(1, 6); // 0.0001% (₹10/crore)

    match ex {
        Exchange::Nfo => {
            let exch = turnover_fee_rate(Exchange::Nfo, SegmentProduct::Futures)
                .unwrap_or_else(|| Decimal::new(183, 7));
            let stt_sell = Decimal::new(2, 4); // 0.02% futures sell-side STT
            let stamp_buy = Decimal::new(2, 5); // 0.002% futures buy-side stamp
            let fees_both = (exch + sebi_one_side) * Decimal::from(2) * gst_mult;
            ((stt_sell + stamp_buy + fees_both) * ten_k).round_dp(4)
        }
        Exchange::Mcx => {
            let exch = turnover_fee_rate(Exchange::Mcx, SegmentProduct::Futures)
                .unwrap_or_else(|| Decimal::new(21, 6));
            let ctt_sell = Decimal::new(1, 4); // 0.01% CTT sell side
            let stamp_buy = Decimal::new(2, 5); // 0.002% stamp buy side
            let fees_both = (exch + sebi_one_side) * Decimal::from(2) * gst_mult;
            ((ctt_sell + stamp_buy + fees_both) * ten_k).round_dp(4)
        }
        Exchange::Cds => {
            let exch = turnover_fee_rate(Exchange::Cds, SegmentProduct::Futures)
                .unwrap_or_else(|| Decimal::new(35, 7));
            let stamp_buy = Decimal::new(1, 5); // 0.001% currency futures stamp
            let fees_both = (exch + sebi_one_side) * Decimal::from(2) * gst_mult;
            ((stamp_buy + fees_both) * ten_k).round_dp(4)
        }
        _ => {
            // Cash equity / ETF delivery round-trip statutory benchmark
            let stt_both = Decimal::new(1, 3) * Decimal::from(2); // 0.1% buy + 0.1% sell
            let exch_both = Decimal::new(297, 7) * Decimal::from(2) * gst_mult;
            let stamp_buy = Decimal::new(15, 5);
            ((stt_both + exch_both + stamp_buy + sebi_one_side * Decimal::from(2)) * ten_k)
                .round_dp(4)
        }
    }
}

/// Applies F&O spread, roll-basis, and statutory fee adjustments across `periods`.
///
/// Mutates `periods` in place so downstream streaks, Markov transitions, distributions,
/// and outliers reflect the active net-of-spread series whenever enabled, while returning
/// side-by-side gross vs net KPIs in [`FrictionSummary`].
pub fn apply_spread_and_basis(
    periods: &mut [SlicedPeriod],
    asset_class: StudyAssetClass,
    exchange_or_symbol: &str,
    calc_kind: ReturnCalcKind,
    cfg: &SpreadCostConfig,
    ma_window: usize,
) -> Option<FrictionSummary> {
    if periods.is_empty() {
        return None;
    }

    let spread_active = cfg.mode == RollBasisMode::NetAfterSpreadAndCosts
        && cfg.spread_slippage_bps > Decimal::ZERO;
    let stat_bps =
        if cfg.mode == RollBasisMode::NetAfterSpreadAndCosts && cfg.include_statutory_costs {
            estimate_statutory_roundtrip_bps(exchange_or_symbol, asset_class)
        } else {
            Decimal::ZERO
        };
    let basis_bps = if cfg.mode != RollBasisMode::GrossSpot {
        cfg.futures_roll_basis_bps
    } else {
        Decimal::ZERO
    };
    let applied_spread_bps = if spread_active {
        cfg.spread_slippage_bps
    } else {
        Decimal::ZERO
    };

    if cfg.mode == RollBasisMode::GrossSpot
        && applied_spread_bps.is_zero()
        && stat_bps.is_zero()
        && basis_bps.is_zero()
    {
        return None;
    }

    let (_, gross_kpis): (_, ReturnKpis) = compute_return_series_and_kpis(periods, ma_window);
    let bps_divisor = Decimal::from(100); // 100 bps = 1%

    let spread_pct = (applied_spread_bps / bps_divisor).round_dp(6);
    let stat_pct = (stat_bps / bps_divisor).round_dp(6);
    let basis_pct = (basis_bps / bps_divisor).round_dp(6);

    let mut cumulative_delta = Decimal::ZERO;
    for p in periods.iter_mut() {
        let gross = p.return_pct;
        let net = match calc_kind {
            ReturnCalcKind::HighToLow => {
                // High-to-Low captures intra-period excursion minus spread & fee slippage.
                (gross - spread_pct - stat_pct).max(Decimal::ZERO)
            }
            ReturnCalcKind::OpenToClose | ReturnCalcKind::CloseToClose => {
                gross + basis_pct - spread_pct - stat_pct
            }
        }
        .round_dp(4);

        cumulative_delta += net - gross;
        p.return_pct = net;
    }

    let (_, net_kpis): (_, ReturnKpis) = compute_return_series_and_kpis(periods, ma_window);

    Some(FrictionSummary {
        mode: cfg.mode,
        gross_avg_return_pct: gross_kpis.avg_return_pct,
        net_avg_return_pct: net_kpis.avg_return_pct,
        gross_sharpe_ratio: gross_kpis.sharpe_ratio,
        net_sharpe_ratio: net_kpis.sharpe_ratio,
        gross_profit_factor: gross_kpis.profit_factor,
        net_profit_factor: net_kpis.profit_factor,
        per_period_spread_bps: applied_spread_bps.round_dp(2),
        per_period_statutory_bps: stat_bps.round_dp(2),
        per_period_basis_bps: basis_bps.round_dp(2),
        cumulative_net_delta_pct: cumulative_delta.round_dp(4),
    })
}
