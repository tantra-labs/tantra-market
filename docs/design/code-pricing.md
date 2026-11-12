# `vyapaar-market-pricing` — Derivatives Pricing Reference

Documents `crates/vyapaar-market-pricing/src/lib.rs`: the `PricingModel` trait
and its `BlackScholesModel` implementation with all five Greeks as exact
`Decimal` values at the API boundary.

## `PricingModel` Trait

```rust
pub trait PricingModel {
    fn price(&self, params: &PriceParams) -> Result<Decimal, PricingError>;
    fn greeks(&self, params: &PriceParams) -> Result<Greeks, PricingError>;
    fn implied_volatility(&self, params: &PriceParams, target_price: Decimal)
        -> Result<Decimal, PricingError>;
}
```

## `PriceParams`

Everything a pricing model needs about one option: `spot`, `strike`,
`time_to_expiry` (years), `risk_free_rate`, `dividend_yield`, `volatility`,
`option_type`. `PriceParams::from_fno_contract` builds these straight from a
parsed `vyapaar_market_finance::FnoContract` (strike/expiry/call-put) + spot
+ as-of date. `validate` enforces strictly-positive spot/strike/expiry/vol.

## `Greeks`

| Field | Convention |
|-------|-----------|
| `price` | Fair value |
| `delta` | `∂price/∂spot` |
| `gamma` | `∂²price/∂spot²` |
| `theta` | Currency units **per year** (divide by 252 for per-day) |
| `vega` | Per 1.0 (100%) of volatility |
| `rho` | Per 1.0 (100%) of rate |

## `BlackScholesModel` (Zero-Floats Exception)

Numerically intensive computation happens internally in `f64` (standard-normal
CDF via Abramowitz & Stegun 26.2.17, error < 7.5e-8); every result is rounded
to `Decimal` (8 dp) at the public API boundary. **No `f64` crosses out of the
crate** — nothing downstream sees a float. `implied_volatility` bisects over
volatility (price is monotone in σ, bounds `1e-4`..`5.0`, 200 iterations).

Errors: `PricingError::{InvalidArgument, NotConverged, NotAnOption}`.

## Related

* `code-finance.md` — the `FnoContract` grammar that feeds `from_fno_contract`.
* `vyapaar-core` — `OptionType`, `Price`.
