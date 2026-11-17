//! Unit tests for [`tantra_market_indicators::GammaProfile`].

use std::str::FromStr;

use rust_decimal::Decimal;
use tantra_market_indicators::{
    GammaInput, GammaProfile, GammaProfileValue, Indicator, StrikeGammaInput,
};

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn input(spot: &str, strikes: Vec<StrikeGammaInput>, lot_size: &str) -> GammaInput {
    GammaInput {
        spot: dec(spot),
        strikes,
        lot_size: dec(lot_size),
    }
}

fn strike(strike: &str, gamma: &str, oi: i64) -> StrikeGammaInput {
    StrikeGammaInput {
        strike: dec(strike),
        gamma: dec(gamma),
        oi: Decimal::from(oi),
    }
}

/// GEX golden vector: `gex = gamma * oi * lot_size * spot` per strike, net is
/// the algebraic sum, and the flip is zero when cumulative GEX never crosses.
///
/// spot = 100, lot_size = 75:
///   95:  0.02 * 10000 * 75 * 100 = 1,500,000
///   100: 0.05 * 8000  * 75 * 100 = 3,000,000
///   105: 0.01 * 6000  * 75 * 100 =   450,000
///   net = 4,950,000; cumulative GEX stays positive -> flip = 0.
#[test]
fn gex_golden_vector_no_flip() {
    let input = input(
        "100",
        vec![
            strike("95", "0.02", 10_000),
            strike("100", "0.05", 8_000),
            strike("105", "0.01", 6_000),
        ],
        "75",
    );
    let mut profile = GammaProfile::new();
    let v = profile.update(input).unwrap();

    let expected = GammaProfileValue {
        net: Decimal::from(4_950_000),
        flip: Decimal::ZERO,
    };
    assert_eq!(v, expected);
    assert_eq!(profile.value(), Some(expected));

    let exposure = profile.exposure().expect("exposure after update");
    assert_eq!(exposure.net, Decimal::from(4_950_000));
    assert_eq!(exposure.per_strike.len(), 3);
    assert_eq!(exposure.per_strike[0].strike.value(), dec("95"));
    assert_eq!(exposure.per_strike[0].gex, Decimal::from(1_500_000));
    assert_eq!(exposure.per_strike[1].gex, Decimal::from(3_000_000));
    assert_eq!(exposure.per_strike[2].gex, Decimal::from(450_000));
    assert_eq!(exposure.total_gex(), Decimal::from(4_950_000));
}

/// A put-heavy chain flips the cumulative GEX: the flip is the strike at which
/// the cumulative sign changes.
///
/// spot = 100, lot_size = 75:
///   95:  +1,500,000 (cum  +1,500,000)
///   100: -2,250,000 (cum    -750,000)  <-- sign change
///   105:   +375,000 (cum    -375,000)
///   net = -375,000; flip = 100.
#[test]
fn gamma_flip_crossing_strike() {
    let input = input(
        "100",
        vec![
            strike("95", "0.02", 10_000),
            strike("100", "-0.03", 10_000),
            strike("105", "0.01", 5_000),
        ],
        "75",
    );
    let mut profile = GammaProfile::new();
    let v = profile.update(input).unwrap();
    assert_eq!(v.net, Decimal::from(-375_000));
    assert_eq!(v.flip, dec("100"));
    assert_eq!(profile.exposure().unwrap().flip, dec("100"));
}

/// Crossings are detected regardless of the input chain order (the shared math
/// sorts by strike internally).
#[test]
fn flip_independent_of_input_order() {
    let unordered = input(
        "100",
        vec![
            strike("105", "0.01", 5_000),
            strike("95", "0.02", 10_000),
            strike("100", "-0.03", 10_000),
        ],
        "75",
    );
    let mut profile = GammaProfile::new();
    let v = profile.update(unordered).unwrap();
    assert_eq!(v.flip, dec("100"));
    assert_eq!(v.net, Decimal::from(-375_000));
}

/// An empty chain nets to zero with no flip.
#[test]
fn empty_chain_is_zero() {
    let input = input("100", vec![], "75");
    let mut profile = GammaProfile::new();
    let v = profile.update(input).unwrap();
    assert_eq!(v.net, Decimal::ZERO);
    assert_eq!(v.flip, Decimal::ZERO);
}

/// `value`/`exposure` are `None` before the first snapshot and cleared on
/// `reset`.
#[test]
fn value_and_reset() {
    let mut profile = GammaProfile::new();
    assert_eq!(profile.value(), None);
    assert!(profile.exposure().is_none());

    let input = input("100", vec![strike("95", "0.02", 10_000)], "75");
    assert!(profile.update(input).is_some());
    assert!(profile.value().is_some());
    assert!(profile.exposure().is_some());

    profile.reset();
    assert_eq!(profile.value(), None);
    assert!(profile.exposure().is_none());
}
