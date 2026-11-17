//! OI-wall support/resistance levels from an OI snapshot.
//!
//! Pure computation: the store read happens in the consumer; this indicator
//! never performs I/O. It picks the top-2 call walls above spot and top-2 put
//! walls below spot by open interest, scores each wall with its normalised
//! delta-OI (`oi_change / oi`, clamped to `[-1, 1]`: building is positive,
//! short-covering is negative), and computes the Local Range PCR over the
//! strikes inside `[ref_low, ref_high]`. All math is exact `Decimal`.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::indicator::Indicator;

/// One strike of an OI snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrikeOi {
    /// Strike price.
    pub strike: Decimal,
    /// Call open interest.
    pub oi_ce: Decimal,
    /// Put open interest.
    pub oi_pe: Decimal,
    /// Call OI change since the previous snapshot.
    pub oi_change_ce: Decimal,
    /// Put OI change since the previous snapshot.
    pub oi_change_pe: Decimal,
}

/// Snapshot input consumed by [`OiLevels`] (by value; no borrowing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OiLevelsInput {
    /// Current spot price.
    pub spot: Decimal,
    /// The full strike chain snapshot.
    pub strikes: Vec<StrikeOi>,
    /// Lower bound of the local range (e.g. `S2`).
    pub ref_low: Decimal,
    /// Upper bound of the local range (e.g. `R2`).
    pub ref_high: Decimal,
}

/// The OI-wall level output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OiLevelsValue {
    /// Max normalised delta-OI score across the top-2 call walls.
    pub resistance_score: Decimal,
    /// Max normalised delta-OI score across the top-2 put walls.
    pub support_score: Decimal,
    /// Local Range PCR = `sum(pe_oi) / sum(ce_oi)` over `[ref_low, ref_high]`;
    /// `0` when there is no CE OI in the range.
    pub local_pcr: Decimal,
    /// Strike of the top call wall, or `0` when no call wall is above spot.
    pub top_oi_resistance: Decimal,
    /// Strike of the top put wall, or `0` when no put wall is below spot.
    pub top_oi_support: Decimal,
}

/// One selected OI wall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OiWall {
    /// Wall strike.
    pub strike: Decimal,
    /// Wall open interest (CE OI for a call wall, PE OI for a put wall).
    pub oi: Decimal,
    /// Normalised delta-OI score in `[-1, 1]`.
    pub score: Decimal,
    /// Wall kind: `"call"` (above spot) or `"put"` (below spot).
    pub kind: String,
}

/// OI-wall / Local-Range-PCR indicator (pure, zero floats, wasm-portable).
#[derive(Debug, Clone, Default)]
pub struct OiLevels {
    /// The selected wall list (up to two calls + two puts), per the last input.
    walls: Vec<OiWall>,
    value: Option<OiLevelsValue>,
}

impl OiLevels {
    /// Creates an empty [`OiLevels`].
    pub fn new() -> Self {
        Self::default()
    }

    /// The full selected wall list, in selection order (calls then puts).
    pub fn walls(&self) -> &[OiWall] {
        &self.walls
    }

    /// Computes the level output from a snapshot without storing state.
    pub fn compute(&mut self, input: OiLevelsInput) -> OiLevelsValue {
        let mut calls: Vec<&StrikeOi> = input
            .strikes
            .iter()
            .filter(|s| s.strike > input.spot)
            .collect();
        calls.sort_by_key(|s| std::cmp::Reverse(s.oi_ce));

        let mut puts: Vec<&StrikeOi> = input
            .strikes
            .iter()
            .filter(|s| s.strike < input.spot)
            .collect();
        puts.sort_by_key(|s| std::cmp::Reverse(s.oi_pe));

        self.walls.clear();
        let mut call_scores = Vec::with_capacity(2);
        let mut top_oi_resistance = Decimal::ZERO;
        for strike in calls.iter().take(2) {
            let score = wall_score(strike.oi_change_ce, strike.oi_ce);
            call_scores.push(score);
            self.walls.push(OiWall {
                strike: strike.strike,
                oi: strike.oi_ce,
                score,
                kind: "call".to_owned(),
            });
            if top_oi_resistance == Decimal::ZERO {
                top_oi_resistance = strike.strike;
            }
        }

        let mut put_scores = Vec::with_capacity(2);
        let mut top_oi_support = Decimal::ZERO;
        for strike in puts.iter().take(2) {
            let score = wall_score(strike.oi_change_pe, strike.oi_pe);
            put_scores.push(score);
            self.walls.push(OiWall {
                strike: strike.strike,
                oi: strike.oi_pe,
                score,
                kind: "put".to_owned(),
            });
            if top_oi_support == Decimal::ZERO {
                top_oi_support = strike.strike;
            }
        }

        let (sum_ce, sum_pe) = input
            .strikes
            .iter()
            .filter(|s| s.strike >= input.ref_low && s.strike <= input.ref_high)
            .fold((Decimal::ZERO, Decimal::ZERO), |(ce, pe), s| {
                (ce + s.oi_ce, pe + s.oi_pe)
            });
        let local_pcr = if sum_ce > Decimal::ZERO {
            sum_pe / sum_ce
        } else {
            Decimal::ZERO
        };

        OiLevelsValue {
            resistance_score: call_scores.into_iter().max().unwrap_or(Decimal::ZERO),
            support_score: put_scores.into_iter().max().unwrap_or(Decimal::ZERO),
            local_pcr,
            top_oi_resistance,
            top_oi_support,
        }
    }
}

/// Normalised delta-OI score: `oi_change / oi` clamped to `[-1, 1]`.
///
/// Positive when OI is building (new positions), negative when OI is
/// unwinding (short-covering / liquidation). Returns `0` when the wall has no
/// open interest.
fn wall_score(oi_change: Decimal, oi: Decimal) -> Decimal {
    if oi == Decimal::ZERO {
        return Decimal::ZERO;
    }
    (oi_change / oi).clamp(-Decimal::ONE, Decimal::ONE)
}

impl Indicator for OiLevels {
    type Input<'a> = OiLevelsInput;
    type Output = OiLevelsValue;

    fn update(&mut self, input: OiLevelsInput) -> Option<OiLevelsValue> {
        let value = self.compute(input);
        self.value = Some(value);
        Some(value)
    }

    fn value(&self) -> Option<OiLevelsValue> {
        self.value
    }

    fn reset(&mut self) {
        self.walls.clear();
        self.value = None;
    }
}
