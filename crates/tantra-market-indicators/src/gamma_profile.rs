//! Per-strike gamma-exposure (GEX) profile plus net and flip.
//!
//! Reuses the exact `GammaExposure` math from `tantra-core`: GEX per strike is
//! `gamma * oi * lot_size * spot` (exact `Decimal`), the net is the algebraic
//! sum, and the flip is the strike at which cumulative GEX changes sign (or
//! `Decimal::ZERO` when it never crosses). Pure computation — the chain
//! snapshot arrives by value from the consumer.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tantra_core::Price;
use tantra_core::options::{GammaExposure, StrikeGamma};

use crate::indicator::Indicator;

/// One strike of a chain snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrikeGammaInput {
    /// Strike price.
    pub strike: Decimal,
    /// Net gamma at this strike (may be negative on the put side).
    pub gamma: Decimal,
    /// Open interest at this strike.
    pub oi: Decimal,
}

/// Chain snapshot consumed by [`GammaProfile`] (by value; no borrowing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaInput {
    /// Current spot price.
    pub spot: Decimal,
    /// The per-strike gamma / OI snapshot.
    pub strikes: Vec<StrikeGammaInput>,
    /// Contract lot size.
    pub lot_size: Decimal,
}

/// The GEX profile output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaProfileValue {
    /// Net (total) GEX across the chain: `sum(gamma * oi * lot * spot)`.
    pub net: Decimal,
    /// The gamma flip: the strike where cumulative GEX changes sign, or
    /// `Decimal::ZERO` when it never crosses.
    pub flip: Decimal,
}

/// Per-strike GEX / gamma-flip indicator (pure, zero floats, wasm-portable).
///
/// The underlying [`GammaExposure`] is kept internally and exposed through
/// [`GammaProfile::exposure`].
#[derive(Debug, Clone, Default)]
pub struct GammaProfile {
    exposure: Option<GammaExposure>,
    value: Option<GammaProfileValue>,
}

impl GammaProfile {
    /// Creates an empty [`GammaProfile`].
    pub fn new() -> Self {
        Self::default()
    }

    /// The full gamma-exposure surface of the last snapshot.
    pub fn exposure(&self) -> Option<&GammaExposure> {
        self.exposure.as_ref()
    }

    /// Computes the profile from a snapshot without storing state.
    pub fn compute(input: GammaInput) -> GammaProfileValue {
        let per_strike: Vec<StrikeGamma> = input
            .strikes
            .iter()
            .map(|s| StrikeGamma {
                strike: Price::new(s.strike),
                gamma: s.gamma,
                oi: s.oi,
                gex: s.gamma * s.oi * input.lot_size * input.spot,
            })
            .collect();
        let exposure = GammaExposure::compute(per_strike);
        GammaProfileValue {
            net: exposure.net,
            flip: exposure.flip,
        }
    }
}

impl Indicator for GammaProfile {
    type Input<'a> = GammaInput;
    type Output = GammaProfileValue;

    fn update(&mut self, input: GammaInput) -> Option<GammaProfileValue> {
        let per_strike: Vec<StrikeGamma> = input
            .strikes
            .iter()
            .map(|s| StrikeGamma {
                strike: Price::new(s.strike),
                gamma: s.gamma,
                oi: s.oi,
                gex: s.gamma * s.oi * input.lot_size * input.spot,
            })
            .collect();
        let exposure = GammaExposure::compute(per_strike);
        let value = GammaProfileValue {
            net: exposure.net,
            flip: exposure.flip,
        };
        self.exposure = Some(exposure);
        self.value = Some(value);
        Some(value)
    }

    fn value(&self) -> Option<GammaProfileValue> {
        self.value
    }

    fn reset(&mut self) {
        self.exposure = None;
        self.value = None;
    }
}
