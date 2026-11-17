//! Unit tests for [`tantra_market_indicators::OiLevels`].

use std::str::FromStr;

use rust_decimal::Decimal;
use tantra_market_indicators::{Indicator, OiLevels, OiLevelsInput, StrikeOi};

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// Golden vector: top-2 call walls above spot, top-2 put walls below spot,
/// delta-OI scores, Local Range PCR and score aggregation.
///
/// Spot = 100; ref range [90, 110].
/// Calls above spot sorted by CE OI: 110 (11000), 115 (6000), 105 (5000).
///   -> top-2 = {110: score -1100/11000 = -0.1, 115: score 3000/6000 = 0.5}
/// Puts below spot sorted by PE OI: 90 (9000), 95 (4000), 85 (2000).
///   -> top-2 = {90: score -900/9000 = -0.1, 95: score 200/4000 = 0.05}
/// PCR over [90, 110] = (9000 + 4000) / (5000 + 11000) = 13000/16000 = 0.8125.
#[test]
fn golden_vector() {
    let strikes = vec![
        StrikeOi {
            strike: dec("85"),
            oi_ce: Decimal::ZERO,
            oi_pe: Decimal::from(2000),
            oi_change_ce: Decimal::ZERO,
            oi_change_pe: Decimal::from(1000),
        },
        StrikeOi {
            strike: dec("90"),
            oi_ce: Decimal::ZERO,
            oi_pe: Decimal::from(9000),
            oi_change_ce: Decimal::ZERO,
            oi_change_pe: Decimal::from(-900),
        },
        StrikeOi {
            strike: dec("95"),
            oi_ce: Decimal::ZERO,
            oi_pe: Decimal::from(4000),
            oi_change_ce: Decimal::ZERO,
            oi_change_pe: Decimal::from(200),
        },
        StrikeOi {
            strike: dec("105"),
            oi_ce: Decimal::from(5000),
            oi_pe: Decimal::ZERO,
            oi_change_ce: Decimal::from(500),
            oi_change_pe: Decimal::ZERO,
        },
        StrikeOi {
            strike: dec("110"),
            oi_ce: Decimal::from(11000),
            oi_pe: Decimal::ZERO,
            oi_change_ce: Decimal::from(-1100),
            oi_change_pe: Decimal::ZERO,
        },
        StrikeOi {
            strike: dec("115"),
            oi_ce: Decimal::from(6000),
            oi_pe: Decimal::ZERO,
            oi_change_ce: Decimal::from(3000),
            oi_change_pe: Decimal::ZERO,
        },
    ];
    let input = OiLevelsInput {
        spot: dec("100"),
        strikes,
        ref_low: dec("90"),
        ref_high: dec("110"),
    };

    let mut oi = OiLevels::new();
    let v = oi.update(input).unwrap();

    assert_eq!(v.resistance_score, dec("0.5"));
    assert_eq!(v.support_score, dec("0.05"));
    assert_eq!(v.local_pcr, dec("0.8125"));
    assert_eq!(v.top_oi_resistance, Decimal::from(110));
    assert_eq!(v.top_oi_support, Decimal::from(90));

    let walls = oi.walls();
    assert_eq!(walls.len(), 4);
    // Calls first (highest OI first), then puts.
    assert_eq!(walls[0].kind, "call");
    assert_eq!(walls[0].strike, Decimal::from(110));
    assert_eq!(walls[0].oi, Decimal::from(11000));
    assert_eq!(walls[0].score, dec("-0.1"));
    assert_eq!(walls[1].kind, "call");
    assert_eq!(walls[1].strike, Decimal::from(115));
    assert_eq!(walls[1].score, dec("0.5"));
    assert_eq!(walls[2].kind, "put");
    assert_eq!(walls[2].strike, Decimal::from(90));
    assert_eq!(walls[2].score, dec("-0.1"));
    assert_eq!(walls[3].kind, "put");
    assert_eq!(walls[3].strike, Decimal::from(95));
    assert_eq!(walls[3].score, dec("0.05"));
}

/// Delta-OI scores keep their sign: building (positive change) is positive,
/// short-covering (negative change) is negative, and the magnitude is clamped
/// to 1.
#[test]
fn delta_oi_score_sign_and_clamp() {
    let strikes = vec![
        StrikeOi {
            strike: dec("105"),
            oi_ce: Decimal::from(100),
            oi_pe: Decimal::ZERO,
            oi_change_ce: Decimal::from(300), // 3x OI -> clamp to 1.0
            oi_change_pe: Decimal::ZERO,
        },
        StrikeOi {
            strike: dec("95"),
            oi_ce: Decimal::ZERO,
            oi_pe: Decimal::from(100),
            oi_change_ce: Decimal::ZERO,
            oi_change_pe: Decimal::from(-200), // -2x OI -> clamp to -1.0
        },
    ];
    let input = OiLevelsInput {
        spot: dec("100"),
        strikes,
        ref_low: dec("90"),
        ref_high: dec("110"),
    };
    let mut oi = OiLevels::new();
    let v = oi.update(input).unwrap();
    assert_eq!(v.resistance_score, Decimal::ONE);
    assert_eq!(v.support_score, -Decimal::ONE);

    let walls = oi.walls();
    assert_eq!(walls[0].score, Decimal::ONE);
    assert_eq!(walls[1].score, -Decimal::ONE);
}

/// A zero-OI wall scores zero (no division-by-zero) and still contributes its
/// strike to the top-wall output.
#[test]
fn zero_oi_wall_scores_zero() {
    let strikes = vec![StrikeOi {
        strike: dec("110"),
        oi_ce: Decimal::ZERO,
        oi_pe: Decimal::ZERO,
        oi_change_ce: Decimal::from(1000),
        oi_change_pe: Decimal::ZERO,
    }];
    let input = OiLevelsInput {
        spot: dec("100"),
        strikes,
        ref_low: dec("90"),
        ref_high: dec("110"),
    };
    let mut oi = OiLevels::new();
    let v = oi.update(input).unwrap();
    assert_eq!(v.resistance_score, Decimal::ZERO);
    assert_eq!(v.top_oi_resistance, Decimal::from(110));
}

/// With no call walls above spot, the resistance side reports zeros and the
/// wall list holds only the puts.
#[test]
fn no_call_walls_above_spot() {
    let strikes = vec![StrikeOi {
        strike: dec("90"),
        oi_ce: Decimal::ZERO,
        oi_pe: Decimal::from(1000),
        oi_change_ce: Decimal::ZERO,
        oi_change_pe: Decimal::from(100),
    }];
    let input = OiLevelsInput {
        spot: dec("100"),
        strikes,
        ref_low: Decimal::ZERO,
        ref_high: Decimal::from(200),
    };
    let mut oi = OiLevels::new();
    let v = oi.update(input).unwrap();
    assert_eq!(v.resistance_score, Decimal::ZERO);
    assert_eq!(v.top_oi_resistance, Decimal::ZERO);
    assert_eq!(v.support_score, dec("0.1"));
    assert_eq!(v.top_oi_support, Decimal::from(90));
    assert_eq!(oi.walls().len(), 1);
    assert_eq!(oi.walls()[0].kind, "put");
}

/// `value` mirrors the last update; `reset` clears both the value and the wall
/// list.
#[test]
fn value_and_reset() {
    let input = OiLevelsInput {
        spot: dec("100"),
        strikes: vec![StrikeOi {
            strike: dec("110"),
            oi_ce: Decimal::from(1000),
            oi_pe: Decimal::ZERO,
            oi_change_ce: Decimal::from(100),
            oi_change_pe: Decimal::ZERO,
        }],
        ref_low: Decimal::ZERO,
        ref_high: Decimal::from(200),
    };
    let mut oi = OiLevels::new();
    assert_eq!(oi.value(), None);
    assert!(oi.update(input).is_some());
    assert!(oi.value().is_some());
    assert_eq!(oi.walls().len(), 1);
    oi.reset();
    assert_eq!(oi.value(), None);
    assert!(oi.walls().is_empty());
}
