//! Per-segment (NFO/MCX/CDS) exchange turnover fee tables (gap 0009, plan
//! 0005): the provider-side answer in `tantra-market-finance::india::costs`
//! that the ledger journal consumes so it stops billing derivative segments at
//! the NSE cash-segment rate. Rates are pinned from the researched circulars
//! (tantra-ledger plan 0005 Appendix A, verified 2026-11-13).

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_core::Exchange;
use tantra_market_finance::india::costs::segments::{SegmentProduct, exchange_turnover_charge};

#[test]
fn nfo_futures_rate_matches_circular() {
    // 100,000 * 0.00183% = 1.83
    assert_eq!(
        exchange_turnover_charge(Exchange::Nfo, SegmentProduct::Futures, dec!(100_000.00)),
        dec!(1.83)
    );
}

#[test]
fn nfo_options_rate_matches_circular() {
    // 100,000 * 0.03553% = 35.53
    assert_eq!(
        exchange_turnover_charge(Exchange::Nfo, SegmentProduct::Options, dec!(100_000.00)),
        dec!(35.53)
    );
}

#[test]
fn mcx_futures_and_options_rates() {
    assert_eq!(
        exchange_turnover_charge(Exchange::Mcx, SegmentProduct::Futures, dec!(100_000.00)),
        dec!(2.10)
    );
    assert_eq!(
        exchange_turnover_charge(Exchange::Mcx, SegmentProduct::Options, dec!(100_000.00)),
        dec!(41.80)
    );
}

#[test]
fn cds_futures_and_options_rates() {
    assert_eq!(
        exchange_turnover_charge(Exchange::Cds, SegmentProduct::Futures, dec!(100_000.00)),
        dec!(0.35)
    );
    assert_eq!(
        exchange_turnover_charge(Exchange::Cds, SegmentProduct::Options, dec!(100_000.00)),
        dec!(31.10)
    );
}

#[test]
fn cash_segments_and_unknown_combinations_yield_zero() {
    assert_eq!(
        exchange_turnover_charge(Exchange::Nse, SegmentProduct::Futures, dec!(100_000.00)),
        Decimal::ZERO
    );
    assert_eq!(
        exchange_turnover_charge(Exchange::Bse, SegmentProduct::Options, dec!(100_000.00)),
        Decimal::ZERO
    );
}

#[test]
fn zero_turnover_yields_zero() {
    assert_eq!(
        exchange_turnover_charge(Exchange::Nfo, SegmentProduct::Futures, Decimal::ZERO),
        Decimal::ZERO
    );
}

#[test]
fn midpoint_rounding_is_banker() {
    // Pin a genuine banker's-rounding tie: 10,000 * 0.03553% = 3.5530 → 3.55
    // (round-half-to-even), not 3.56.
    assert_eq!(
        exchange_turnover_charge(Exchange::Nfo, SegmentProduct::Options, dec!(10_000.00)),
        dec!(3.55)
    );
}
