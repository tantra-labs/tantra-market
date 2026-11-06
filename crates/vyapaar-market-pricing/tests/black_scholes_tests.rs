//! Golden-vector and invariant tests for `vyapaar-market-pricing`.
//!
//! Reference values are independent `f64` computations (see the crate's
//! float-internal / Decimal-boundary note). Assertions use a 3–4 decimal
//! tolerance because the internal standard-normal CDF is the classic
//! Abramowitz & Stegun approximation (error < 7.5e-8).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use vyapaar_core::{OptionType, Price};
use vyapaar_market_finance::parse_fno_symbol;
use vyapaar_market_pricing::{BlackScholesModel, PriceParams, PricingError, PricingModel};

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn params(
    spot: &str,
    strike: &str,
    t: &str,
    r: &str,
    q: &str,
    sigma: &str,
    ot: OptionType,
) -> PriceParams {
    PriceParams {
        spot: Price::from_rupees(spot).unwrap(),
        strike: Price::from_rupees(strike).unwrap(),
        time_to_expiry: d(t),
        risk_free_rate: d(r),
        dividend_yield: d(q),
        volatility: d(sigma),
        option_type: ot,
    }
}

// ---------------------------------------------------------------------------
// Golden vectors
// ---------------------------------------------------------------------------

#[test]
fn black_scholes_call_golden() {
    // S=100, K=100, T=1, r=0.05, q=0, σ=0.2.
    let model = BlackScholesModel;
    let g = model
        .greeks(&params(
            "100",
            "100",
            "1",
            "0.05",
            "0",
            "0.2",
            OptionType::Call,
        ))
        .unwrap();
    assert_eq!(g.price.round_dp(4), d("10.4506"));
    assert_eq!(g.delta.round_dp(3), d("0.637"));
    assert_eq!(g.gamma.round_dp(3), d("0.019"));
    assert_eq!(g.theta.round_dp(3), d("-6.414"));
    assert_eq!(g.vega.round_dp(3), d("37.524"));
    assert_eq!(g.rho.round_dp(3), d("53.232"));
}

#[test]
fn black_scholes_put_golden() {
    // S=100, K=100, T=1, r=0.05, q=0, σ=0.2.
    let model = BlackScholesModel;
    let g = model
        .greeks(&params(
            "100",
            "100",
            "1",
            "0.05",
            "0",
            "0.2",
            OptionType::Put,
        ))
        .unwrap();
    assert_eq!(g.price.round_dp(4), d("5.5735"));
    assert_eq!(g.delta.round_dp(3), d("-0.363"));
    assert_eq!(g.gamma.round_dp(3), d("0.019"));
    assert_eq!(g.theta.round_dp(3), d("-1.658"));
    assert_eq!(g.vega.round_dp(3), d("37.524"));
    assert_eq!(g.rho.round_dp(3), d("-41.890"));
}

#[test]
fn black_scholes_with_dividend_yield_golden() {
    // S=110, K=100, T=0.5, r=0.06, q=0.02, σ=0.3.
    let model = BlackScholesModel;
    let call = model
        .greeks(&params(
            "110",
            "100",
            "0.5",
            "0.06",
            "0.02",
            "0.3",
            OptionType::Call,
        ))
        .unwrap();
    assert_eq!(call.price.round_dp(4), d("15.8767"));
    assert_eq!(call.delta.round_dp(3), d("0.735"));
    assert_eq!(call.gamma.round_dp(3), d("0.014"));
    assert_eq!(call.rho.round_dp(3), d("32.468"));

    let put = model
        .greeks(&params(
            "110",
            "100",
            "0.5",
            "0.06",
            "0.02",
            "0.3",
            OptionType::Put,
        ))
        .unwrap();
    assert_eq!(put.price.round_dp(4), d("4.0158"));
    assert_eq!(put.delta.round_dp(3), d("-0.255"));
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

#[test]
fn put_call_parity_holds() {
    let model = BlackScholesModel;
    let call = model
        .greeks(&params(
            "100",
            "100",
            "1",
            "0.05",
            "0",
            "0.2",
            OptionType::Call,
        ))
        .unwrap();
    let put = model
        .greeks(&params(
            "100",
            "100",
            "1",
            "0.05",
            "0",
            "0.2",
            OptionType::Put,
        ))
        .unwrap();
    // c − p = S·e^(−qT) − K·e^(−rT) = 100 − 100·e^(−0.05) = 4.87705755…
    let expected = d("100") - d("100") * d("0.9512294245");
    let actual = call.price - put.price;
    assert!(
        (actual - expected).abs() < d("0.0005"),
        "parity violated: c − p = {actual}, expected ≈ {expected}"
    );
}

#[test]
fn delta_is_bounded_between_negative_one_and_one() {
    let model = BlackScholesModel;
    let call = model
        .greeks(&params(
            "100",
            "100",
            "1",
            "0.05",
            "0",
            "0.2",
            OptionType::Call,
        ))
        .unwrap();
    let put = model
        .greeks(&params(
            "100",
            "100",
            "1",
            "0.05",
            "0",
            "0.2",
            OptionType::Put,
        ))
        .unwrap();
    assert!(call.delta > d("0") && call.delta < d("1"));
    assert!(put.delta > d("-1") && put.delta < d("0"));
}

#[test]
fn deep_itm_call_is_worth_at_least_intrinsic_value() {
    let model = BlackScholesModel;
    let g = model
        .greeks(&params(
            "200",
            "50",
            "1",
            "0.05",
            "0",
            "0.3",
            OptionType::Call,
        ))
        .unwrap();
    let intrinsic = d("200") - d("50") * d("0.9512294245"); // 152.44…
    assert!(g.price >= intrinsic - d("0.001"));
    assert!(g.price < d("200"));
}

#[test]
fn deep_otm_call_is_tiny_but_positive() {
    let model = BlackScholesModel;
    let price = model
        .price(&params(
            "50",
            "200",
            "1",
            "0.05",
            "0",
            "0.3",
            OptionType::Call,
        ))
        .unwrap();
    assert!(price > d("0"));
    assert!(price < d("0.10"));
}

// ---------------------------------------------------------------------------
// Implied volatility
// ---------------------------------------------------------------------------

#[test]
fn implied_volatility_round_trips_the_market_price() {
    let model = BlackScholesModel;
    let ps = params("100", "100", "1", "0.05", "0", "0.2", OptionType::Call);
    let price = model.price(&ps).unwrap();
    let iv = model.implied_volatility(&ps, price).unwrap();
    assert_eq!(iv.round_dp(4), d("0.2"));
}

#[test]
fn implied_volatility_put_side_round_trips() {
    let model = BlackScholesModel;
    let ps = params("110", "100", "0.5", "0.06", "0.02", "0.3", OptionType::Put);
    let price = model.price(&ps).unwrap();
    let iv = model.implied_volatility(&ps, price).unwrap();
    assert_eq!(iv.round_dp(4), d("0.3"));
}

#[test]
fn implied_volatility_out_of_range_price_does_not_converge() {
    let model = BlackScholesModel;
    let ps = params("100", "100", "1", "0.05", "0", "0.2", OptionType::Call);
    // A target below the σ→0 (intrinsic-value) price is unachievable.
    assert_eq!(
        model
            .implied_volatility(&ps, d("0.0000000001"))
            .unwrap_err(),
        PricingError::NotConverged
    );
}

// ---------------------------------------------------------------------------
// Validation errors
// ---------------------------------------------------------------------------

#[test]
fn non_positive_parameters_are_rejected() {
    let model = BlackScholesModel;
    assert!(matches!(
        model.price(&params("0", "100", "1", "0.05", "0", "0.2", OptionType::Call)),
        Err(PricingError::InvalidArgument(msg)) if msg.contains("spot")
    ));
    assert!(matches!(
        model.price(&params("100", "0", "1", "0.05", "0", "0.2", OptionType::Call)),
        Err(PricingError::InvalidArgument(msg)) if msg.contains("strike")
    ));
    assert!(matches!(
        model.price(&params("100", "100", "0", "0.05", "0", "0.2", OptionType::Call)),
        Err(PricingError::InvalidArgument(msg)) if msg.contains("time_to_expiry")
    ));
    assert!(matches!(
        model.price(&params("100", "100", "1", "0.05", "0", "0", OptionType::Call)),
        Err(PricingError::InvalidArgument(msg)) if msg.contains("volatility")
    ));
}

// ---------------------------------------------------------------------------
// F&O contract convenience constructor
// ---------------------------------------------------------------------------

#[test]
fn from_fno_contract_builds_parameters_and_prices() {
    let contract = parse_fno_symbol("NIFTY24OCT24000CE").unwrap();
    let as_of = NaiveDate::from_ymd_opt(2024, 9, 30).unwrap();
    let ps = PriceParams::from_fno_contract(
        &contract,
        Price::from_rupees("24500").unwrap(),
        as_of,
        d("0.06"),
        d("0"),
        d("0.15"),
    )
    .unwrap();

    // Strike, type and approximate time-to-expiry from the contract grammar.
    assert_eq!(ps.strike, Price::from_rupees("24000").unwrap());
    assert_eq!(ps.option_type, OptionType::Call);
    // Expiry is 2024-10-31 (last Thursday of Oct 2024), 31 days out.
    assert_eq!(ps.time_to_expiry.round_dp(6), d("0.084932"));

    let model = BlackScholesModel;
    let g = model.greeks(&ps).unwrap();
    assert!(g.price > d("500"));
    assert!(g.delta > d("0") && g.delta < d("1"));
}

#[test]
fn from_fno_contract_rejects_futures_and_expired_contracts() {
    let future = parse_fno_symbol("NIFTY24OCTFUT").unwrap();
    let as_of = NaiveDate::from_ymd_opt(2024, 9, 30).unwrap();
    assert_eq!(
        PriceParams::from_fno_contract(
            &future,
            Price::from_rupees("24500").unwrap(),
            as_of,
            d("0.06"),
            d("0"),
            d("0.15"),
        )
        .unwrap_err(),
        PricingError::NotAnOption
    );

    // Same contract priced after its expiry date.
    let option = parse_fno_symbol("NIFTY24OCT24000CE").unwrap();
    let expired = NaiveDate::from_ymd_opt(2024, 11, 15).unwrap();
    assert!(matches!(
        PriceParams::from_fno_contract(
            &option,
            Price::from_rupees("24500").unwrap(),
            expired,
            d("0.06"),
            d("0"),
            d("0.15"),
        ),
        Err(PricingError::InvalidArgument(msg)) if msg.contains("already expired")
    ));
}
