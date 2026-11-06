#![deny(unsafe_code)]

//! # व्यापार・मूल्यन vyapaar-market-pricing
//!
//! Derivatives pricing for the Vyapaar market-data subsystem: the
//! [`PricingModel`] trait and an initial [`BlackScholesModel`] implementation,
//! with all five classic Greeks surfaced as exact `Decimal` values at the API
//! boundary.
//!
//! This is the pricing layer *on top of* `vyapaar-market-finance`'s F&O
//! contract grammar: [`PriceParams::from_fno_contract`] turns a parsed
//! [`FnoContract`] (strike, expiry, call/put) plus a spot into a full pricing
//! input. The full Fincept stack — 14+ exotic option types, SABR/Heston
//! calibration, vol surfaces, swaps — is explicitly out of scope for this
//! crate's first slice.
//!
//! # Zero-floats policy note (documented exception)
//!
//! Black-Scholes closed-form evaluation (and the implied-volatility root
//! finder) is conventionally done in floating point for numerical stability
//! and solver convergence, which conflicts with the workspace's zero-floats
//! policy. This crate resolves that the way `vyapaar-market/docs/plans/0001`
//! §2.2 suggests:
//!
//! - all **numerically intensive computation happens internally in `f64`**
//!   (isolated in this module);
//! - every result is **rounded to `Decimal` (8 decimal places) at the public
//!   API boundary**;
//! - no `f64` value ever crosses out of this crate — nothing downstream sees
//!   a float, and no domain aggregate in `vyapaar-ledger`/`vyapaar-risk` is
//!   ever fed a float from here.
//!
//! # Greeks conventions
//!
//! - `theta` is in currency units **per year** (matching `time_to_expiry` in
//!   years; divide by 252 for a per-trading-day figure).
//! - `vega` is per **1.0 (100 percentage points) of volatility**; divide by
//!   100 for the per-1%-vol convention.
//! - `rho` is per 1.0 (100%) of rate.

use std::f64::consts::PI;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use thiserror::Error;
use vyapaar_core::{OptionType, Price};
use vyapaar_market_finance::FnoContract;

/// Pricing errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PricingError {
    /// A parameter is outside its valid domain (non-positive spot/strike/
    /// expiry/volatility, expired contract).
    #[error("invalid pricing parameters: {0}")]
    InvalidArgument(String),

    /// The implied-volatility root finder failed to converge or the target
    /// price is outside the achievable range.
    #[error("implied-volatility solver did not converge")]
    NotConverged,

    /// A futures contract (no strike) was passed where an option is required.
    #[error("the contract is not an option (no strike)")]
    NotAnOption,
}

/// Everything a pricing model needs about one option.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceParams {
    /// Current price of the underlying.
    pub spot: Price,
    /// Strike price.
    pub strike: Price,
    /// Time to expiry in years (must be strictly positive).
    pub time_to_expiry: Decimal,
    /// Continuously-compounded annualized risk-free rate (e.g. `0.07`).
    pub risk_free_rate: Decimal,
    /// Continuously-compounded annualized dividend yield (e.g. `0.02`).
    pub dividend_yield: Decimal,
    /// Annualized volatility (e.g. `0.20` for 20%).
    pub volatility: Decimal,
    /// Call or put.
    pub option_type: OptionType,
}

impl PriceParams {
    /// Builds pricing parameters from a parsed F&O option contract, a spot
    /// price, and an as-of date. Time to expiry is the calendar days between
    /// `as_of` and the contract's expiry date, divided by 365 (a documented
    /// approximation; day-count conventions are a follow-on refinement).
    pub fn from_fno_contract(
        contract: &FnoContract,
        spot: Price,
        as_of: NaiveDate,
        risk_free_rate: Decimal,
        dividend_yield: Decimal,
        volatility: Decimal,
    ) -> Result<Self, PricingError> {
        let strike = contract.strike().ok_or(PricingError::NotAnOption)?;
        let option_type = match contract.option_type() {
            Some(vyapaar_market_finance::india::fno::OptionType::Call) => OptionType::Call,
            Some(vyapaar_market_finance::india::fno::OptionType::Put) => OptionType::Put,
            None => return Err(PricingError::NotAnOption),
        };
        let expiry = contract.expiry().expiry_date().ok_or_else(|| {
            PricingError::InvalidArgument("contract expiry cannot be resolved".into())
        })?;
        let days = expiry.signed_duration_since(as_of).num_days();
        if days <= 0 {
            return Err(PricingError::InvalidArgument(
                "contract has already expired at the given as-of date".into(),
            ));
        }
        let time_to_expiry = Decimal::from(days) / Decimal::from(365);
        Ok(Self {
            spot,
            strike,
            time_to_expiry,
            risk_free_rate,
            dividend_yield,
            volatility,
            option_type,
        })
    }

    /// Validates the parameter domain. `time_to_expiry` and `volatility` must
    /// be strictly positive, and spot/strike strictly positive.
    pub fn validate(&self) -> Result<(), PricingError> {
        if self.spot.value() <= Decimal::ZERO {
            return Err(PricingError::InvalidArgument(
                "spot must be strictly positive".into(),
            ));
        }
        if self.strike.value() <= Decimal::ZERO {
            return Err(PricingError::InvalidArgument(
                "strike must be strictly positive".into(),
            ));
        }
        if self.time_to_expiry <= Decimal::ZERO {
            return Err(PricingError::InvalidArgument(
                "time_to_expiry must be strictly positive".into(),
            ));
        }
        if self.volatility <= Decimal::ZERO {
            return Err(PricingError::InvalidArgument(
                "volatility must be strictly positive".into(),
            ));
        }
        Ok(())
    }
}

/// The Greeks of an option, plus its fair value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Greeks {
    /// Fair value of the option.
    pub price: Decimal,
    /// `∂price/∂spot`.
    pub delta: Decimal,
    /// `∂²price/∂spot²`.
    pub gamma: Decimal,
    /// `∂price/∂time`, currency units per year.
    pub theta: Decimal,
    /// `∂price/∂volatility`, per 1.0 (100%) of volatility.
    pub vega: Decimal,
    /// `∂price/∂rate`, per 1.0 (100%) of rate.
    pub rho: Decimal,
}

/// A derivatives pricing model.
pub trait PricingModel {
    /// Fair value of the option.
    fn price(&self, params: &PriceParams) -> Result<Decimal, PricingError>;

    /// Fair value plus all five Greeks.
    fn greeks(&self, params: &PriceParams) -> Result<Greeks, PricingError>;

    /// The volatility that reproduces `target_price` for the given parameters
    /// (implied volatility), found by root-finding over the pricing formula.
    fn implied_volatility(
        &self,
        params: &PriceParams,
        target_price: Decimal,
    ) -> Result<Decimal, PricingError>;
}

/// Black-Scholes closed-form pricing.
///
/// `f64` internally; every returned value is rounded to `Decimal` at the API
/// boundary (see the module-level zero-floats note).
#[derive(Debug, Clone, Copy, Default)]
pub struct BlackScholesModel;

/// The validated `f64` working set for one pricing call (internal only — no
/// float crosses the public API).
#[derive(Debug, Clone, Copy)]
struct F64Inputs {
    spot: f64,
    strike: f64,
    time: f64,
    rate: f64,
    dividend: f64,
    vol: f64,
    is_call: bool,
}

impl BlackScholesModel {
    /// Converts validated parameters into the `f64` working set.
    fn f64_inputs(&self, params: &PriceParams) -> Result<F64Inputs, PricingError> {
        params.validate()?;
        Ok(F64Inputs {
            spot: f64(params.spot.value(), "spot")?,
            strike: f64(params.strike.value(), "strike")?,
            time: f64(params.time_to_expiry, "time_to_expiry")?,
            rate: f64(params.risk_free_rate, "risk_free_rate")?,
            dividend: f64(params.dividend_yield, "dividend_yield")?,
            vol: f64(params.volatility, "volatility")?,
            is_call: params.option_type == OptionType::Call,
        })
    }
}

/// `f64` standard normal CDF (Abramowitz & Stegun 26.2.17; error < 7.5e-8).
fn norm_cdf(x: f64) -> f64 {
    const B1: f64 = 0.319_381_530;
    const B2: f64 = -0.356_563_782;
    const B3: f64 = 1.781_477_937;
    const B4: f64 = -1.821_255_978;
    const B5: f64 = 1.330_274_429;
    const P: f64 = 0.231_641_9;
    if x >= 0.0 {
        let t = 1.0 / (1.0 + P * x);
        let poly = t * (B1 + t * (B2 + t * (B3 + t * (B4 + t * B5))));
        1.0 - norm_pdf(x) * poly
    } else {
        1.0 - norm_cdf(-x)
    }
}

/// `f64` standard normal PDF.
fn norm_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * PI).sqrt()
}

/// Pure `f64` Black-Scholes price.
fn bs_price_f64(s: f64, k: f64, t: f64, r: f64, q: f64, sigma: f64, is_call: bool) -> f64 {
    let (d1, d2) = bs_d1_d2(s, k, t, r, q, sigma);
    if is_call {
        s * (-q * t).exp() * norm_cdf(d1) - k * (-r * t).exp() * norm_cdf(d2)
    } else {
        k * (-r * t).exp() * norm_cdf(-d2) - s * (-q * t).exp() * norm_cdf(-d1)
    }
}

/// `d1 = (ln(S/K) + (r − q + σ²/2)T) / (σ√T)`, `d2 = d1 − σ√T`.
fn bs_d1_d2(s: f64, k: f64, t: f64, r: f64, q: f64, sigma: f64) -> (f64, f64) {
    let sqrt_t = t.sqrt();
    let d1 = ((s / k).ln() + (r - q + 0.5 * sigma * sigma) * t) / (sigma * sqrt_t);
    let d2 = d1 - sigma * sqrt_t;
    (d1, d2)
}

/// Rounds an `f64` into an 8-decimal `Decimal` at the API boundary.
fn to_decimal(value: f64) -> Result<Decimal, PricingError> {
    if !value.is_finite() {
        return Err(PricingError::InvalidArgument(
            "computation produced a non-finite value".into(),
        ));
    }
    Decimal::from_f64_retain(value)
        .map(|d| d.round_dp(8))
        .ok_or_else(|| {
            PricingError::InvalidArgument("computation produced a non-finite value".into())
        })
}

/// Converts a validated `Decimal` parameter into its `f64` working copy.
fn f64(value: Decimal, name: &'static str) -> Result<f64, PricingError> {
    value
        .to_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| PricingError::InvalidArgument(format!("{name} is not representable")))
}

impl PricingModel for BlackScholesModel {
    fn price(&self, params: &PriceParams) -> Result<Decimal, PricingError> {
        let F64Inputs {
            spot: s,
            strike: k,
            time: t,
            rate: r,
            dividend: q,
            vol: sigma,
            is_call,
        } = self.f64_inputs(params)?;
        to_decimal(bs_price_f64(s, k, t, r, q, sigma, is_call))
    }

    fn greeks(&self, params: &PriceParams) -> Result<Greeks, PricingError> {
        let F64Inputs {
            spot: s,
            strike: k,
            time: t,
            rate: r,
            dividend: q,
            vol: sigma,
            is_call,
        } = self.f64_inputs(params)?;
        let (d1, d2) = bs_d1_d2(s, k, t, r, q, sigma);
        let sqrt_t = t.sqrt();
        let pdf_d1 = norm_pdf(d1);
        let nd1 = norm_cdf(d1);
        let nd2 = norm_cdf(d2);

        let price = bs_price_f64(s, k, t, r, q, sigma, is_call);
        let gamma = (-q * t).exp() * pdf_d1 / (s * sigma * sqrt_t);
        let vega = s * (-q * t).exp() * pdf_d1 * sqrt_t;
        let (delta, theta, rho) = if is_call {
            let delta = (-q * t).exp() * nd1;
            let theta = -(-q * t).exp() * s * pdf_d1 * sigma / (2.0 * sqrt_t)
                - r * k * (-r * t).exp() * nd2
                + q * s * (-q * t).exp() * nd1;
            let rho = k * t * (-r * t).exp() * nd2;
            (delta, theta, rho)
        } else {
            let delta = (-q * t).exp() * (nd1 - 1.0);
            let theta = -(-q * t).exp() * s * pdf_d1 * sigma / (2.0 * sqrt_t)
                + r * k * (-r * t).exp() * norm_cdf(-d2)
                - q * s * (-q * t).exp() * norm_cdf(-d1);
            let rho = -k * t * (-r * t).exp() * norm_cdf(-d2);
            (delta, theta, rho)
        };

        Ok(Greeks {
            price: to_decimal(price)?,
            delta: to_decimal(delta)?,
            gamma: to_decimal(gamma)?,
            theta: to_decimal(theta)?,
            vega: to_decimal(vega)?,
            rho: to_decimal(rho)?,
        })
    }

    fn implied_volatility(
        &self,
        params: &PriceParams,
        target_price: Decimal,
    ) -> Result<Decimal, PricingError> {
        let F64Inputs {
            spot: s,
            strike: k,
            time: t,
            rate: r,
            dividend: q,
            vol: _,
            is_call,
        } = self.f64_inputs(params)?;
        let target = f64(target_price, "target_price")?;
        if target <= 0.0 {
            return Err(PricingError::InvalidArgument(
                "target_price must be strictly positive".into(),
            ));
        }

        // Bisection over volatility: price is monotone increasing in σ.
        let mut lo = 1e-4;
        let mut hi = 5.0;
        let price_lo = bs_price_f64(s, k, t, r, q, lo, is_call);
        let price_hi = bs_price_f64(s, k, t, r, q, hi, is_call);
        if target < price_lo || target > price_hi {
            return Err(PricingError::NotConverged);
        }
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            let price = bs_price_f64(s, k, t, r, q, mid, is_call);
            if (price - target).abs() < 1e-9 {
                return to_decimal(mid);
            }
            if price < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Err(PricingError::NotConverged)
    }
}
