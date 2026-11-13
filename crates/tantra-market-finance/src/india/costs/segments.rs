//! Per-segment (NFO/MCX/CDS) exchange turnover fee tables.
//!
//! The provider-side answer to `tantra-ledger` plan 0005 gap 0009: the
//! ledger's `TaxCalculator::compute_charges` used to bill every derivative
//! segment at the NSE cash-segment rate. These exact `Decimal` tables carry the
//! researched exchange transaction charges (source of truth in the plan's
//! Appendix A, verified 2026-11-13) so the domain heart can look the segment
//! rate up instead of falling back.
//!
//! Zero-float by design: rates are exact `rust_decimal::Decimal` fractions of
//! notional (futures) or premium (options), per side, and include the
//! 2026-03-01 NSE transaction-charge / IPFT re-split:
//!
//! | Segment | Futures (per side) | Options (per side, on premium) |
//! |---------|--------------------|--------------------------------|
//! | NFO     | 0.00183% (₹183/cr) | 0.03553% (₹3,553/cr)           |
//! | MCX     | 0.00210% (₹2.10/l) | 0.04180% (₹41.80/l)            |
//! | CDS     | 0.00035% (₹35/cr)  | 0.03110% (₹3,110/cr)           |

use rust_decimal::Decimal;
use tantra_core::Exchange;

/// Derivative product axis for the per-segment fee tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SegmentProduct {
    /// Futures contract: charged on contract value (notional turnover).
    Futures,
    /// Options contract: charged on premium turnover.
    Options,
}

/// NFO (NSE equity derivatives) futures transaction charge, per side:
/// ₹182.99/crore transaction charge + ₹0.01/crore IPFT (NSE/FA/73061,
/// effective 2026-03-01) = 0.00183%.
pub fn nfo_futures_rate() -> Decimal {
    Decimal::new(183, 7)
}
/// NFO equity options transaction charge, per side, on premium:
/// ₹3,552.99/crore + ₹0.01/crore IPFT = 0.03553%.
pub fn nfo_options_rate() -> Decimal {
    Decimal::new(3553, 7)
}
/// MCX commodity futures transaction charge, per side:
/// ₹2.10/lakh of turnover (MCX/F&A/631/2024) = 0.0021%.
pub fn mcx_futures_rate() -> Decimal {
    Decimal::new(21, 6)
}
/// MCX commodity options transaction charge, per side, on premium:
/// ₹41.80/lakh (MCX/F&A/631/2024) = 0.0418%.
pub fn mcx_options_rate() -> Decimal {
    Decimal::new(418, 6)
}
/// CDS (NSE currency derivatives) futures transaction charge, per side:
/// ₹0.35/lakh (NSE/FA/64232) = 0.00035%.
pub fn cds_futures_rate() -> Decimal {
    Decimal::new(35, 7)
}
/// CDS currency options transaction charge, per side, on premium:
/// ₹31.10/lakh (NSE/FA/64232) = 0.0311%.
pub fn cds_options_rate() -> Decimal {
    Decimal::new(311, 6)
}

/// Canonical rounding for fee charges: round to nearest 2 dp (banker's /
/// round-half-to-even via `Decimal::round_dp`), the same rule the ledger's
/// canonical track uses.
pub fn round_inr(amount: Decimal) -> Decimal {
    amount.round_dp(2)
}

/// The per-side transaction charge rate for a derivative segment and product,
/// as a fraction of notional (futures) or premium (options).
///
/// Returns `None` for the cash segments (`Nse`, `Bse`) — the ledger keeps its
/// own cash-segment rates — and `None` for a delivery/intraday product on a
/// derivative exchange.
pub fn turnover_fee_rate(exchange: Exchange, product: SegmentProduct) -> Option<Decimal> {
    let rate = match (exchange, product) {
        (Exchange::Nfo, SegmentProduct::Futures) => nfo_futures_rate(),
        (Exchange::Nfo, SegmentProduct::Options) => nfo_options_rate(),
        (Exchange::Mcx, SegmentProduct::Futures) => mcx_futures_rate(),
        (Exchange::Mcx, SegmentProduct::Options) => mcx_options_rate(),
        (Exchange::Cds, SegmentProduct::Futures) => cds_futures_rate(),
        (Exchange::Cds, SegmentProduct::Options) => cds_options_rate(),
        _ => return None,
    };
    Some(rate)
}

/// The exact exchange transaction charge for a derivative segment trade, given
/// its turnover (contract value for futures, premium value for options).
///
/// Zero for a non-positive turnover or an unknown segment/product combination.
pub fn exchange_turnover_charge(
    exchange: Exchange,
    product: SegmentProduct,
    turnover: Decimal,
) -> Decimal {
    if turnover <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    match turnover_fee_rate(exchange, product) {
        Some(rate) => round_inr(turnover * rate),
        None => Decimal::ZERO,
    }
}
