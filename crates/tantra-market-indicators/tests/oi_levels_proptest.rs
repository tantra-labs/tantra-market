//! Proptest invariants for [`tantra_market_indicators::OiLevels`]: selected
//! walls stay on the correct side of spot, the Local Range PCR is never
//! negative, and every wall score stays in `[-1, 1]`.

use proptest::collection::vec as prop_vec;
use proptest::prelude::*;
use rust_decimal::Decimal;
use tantra_market_indicators::{Indicator, OiLevels, OiLevelsInput, StrikeOi};

fn dec_from(v: (u32, u32, u32, i32, i32)) -> StrikeOi {
    StrikeOi {
        strike: Decimal::from(v.0),
        oi_ce: Decimal::from(v.1),
        oi_pe: Decimal::from(v.2),
        oi_change_ce: Decimal::from(v.3),
        oi_change_pe: Decimal::from(v.4),
    }
}

proptest! {
    /// Walls selected by `OiLevels` always stay on their side of spot, the
    /// delta-OI scores stay within `[-1, 1]`, and the Local Range PCR is never
    /// negative.
    #[test]
    fn walls_and_pcr_invariants(
        spot in 1u32..=1000u32,
        ref_low in 0u32..=1000u32,
        ref_high in 0u32..=1000u32,
        strikes in prop_vec(
            (1u32..=1000u32, 0u32..=1000u32, 0u32..=1000u32, -1000i32..=1000i32, -1000i32..=1000i32),
            0..12,
        ),
    ) {
        let low = ref_low.min(ref_high);
        let high = ref_low.max(ref_high);
        let input = OiLevelsInput {
            spot: Decimal::from(spot),
            strikes: strikes.iter().copied().map(dec_from).collect(),
            ref_low: Decimal::from(low),
            ref_high: Decimal::from(high),
        };
        let mut oi = OiLevels::new();
        let v = oi.update(input).unwrap();

        prop_assert!(v.local_pcr >= Decimal::ZERO, "PCR must not be negative: {v:?}");
        prop_assert!(
            (v.resistance_score >= -Decimal::ONE && v.resistance_score <= Decimal::ONE),
            "resistance score out of [-1, 1]: {:?}",
            v.resistance_score,
        );
        prop_assert!(
            (v.support_score >= -Decimal::ONE && v.support_score <= Decimal::ONE),
            "support score out of [-1, 1]: {:?}",
            v.support_score,
        );

        let spot_dec = Decimal::from(spot);
        if v.top_oi_resistance != Decimal::ZERO {
            prop_assert!(v.top_oi_resistance > spot_dec, "call wall must be above spot");
        }
        if v.top_oi_support != Decimal::ZERO {
            prop_assert!(v.top_oi_support < spot_dec, "put wall must be below spot");
        }

        for wall in oi.walls() {
            prop_assert!(
                (wall.score >= -Decimal::ONE && wall.score <= Decimal::ONE),
                "wall score out of [-1, 1]: {:?}",
                wall.score,
            );
            match wall.kind.as_str() {
                "call" => {
                    prop_assert!(wall.strike > spot_dec, "call wall below spot: {wall:?}");
                }
                "put" => {
                    prop_assert!(wall.strike < spot_dec, "put wall above spot: {wall:?}");
                }
                other => prop_assert!(false, "unknown wall kind: {other}"),
            }
        }
    }
}
