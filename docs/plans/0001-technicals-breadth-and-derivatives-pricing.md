# 0001 — Technical Indicator Breadth, Market Data Scope, and Derivatives Pricing

> **Status**: Proposed — not yet implemented.
> **Target Crates**: `vyapaar-market-indicators` (expand), `vyapaar-market-pricing` (new)
> **Depends on reading**:
> - (none external)

---

## 0. Executive Summary & Objective

A research pass comparing Vyapaar against Fincept (docs.fincept.in) surfaced three related but distinct gaps in this repo's market-data subsystem: (a) `vyapaar-market-indicators` implements roughly 17 incremental indicators against Fincept's 201-indicator catalog — a breadth gap, not an architectural one; (b) `vyapaar-market-finance` already encodes India F&O contract grammar (expiry rules, lot sizes, strike conventions) but has no pricing layer on top of it — no options Greeks, no implied vol, no yield-curve or bond pricing — where Fincept's Pricing engine covers bonds, credit, yield curves, options Greeks, vol surfaces, and swaps; (c) Vyapaar's market data is India-only, driven by 5 broker adapters, versus Fincept's global/68-country coverage. This document scopes (a) and (b) as concrete, phased work, and explicitly surfaces (c) as a deliberate scope decision rather than a gap to auto-close.

---

## 1. Context

- `vyapaar-market-indicators` is described in its own crate metadata as "Incremental technical indicators over price and bar series (pure, WASM-portable)". It currently implements: SMA, EMA, RSI, MACD, Bollinger Bands, ATR, ADX, Stochastic, Supertrend, VWAP, Williams %R, OBV, Donchian Channels, CPR, RMA (~17 indicators). The incremental/pure/WASM-portable design is already the correct shape per the workspace's retargettable-design invariants (§4.3 of the workspace `AGENTS.md`) — this is a breadth problem, not a redesign.
- `vyapaar-market-finance` is described as providing "Generic market contracts, market packs, and registry — the canonical India market microstructure pack (MarketProfile, NseCalendar, statutory cost schedules, F&O grammar)". It owns contract *grammar* (what a valid NFO option symbol looks like, expiry cycles, lot sizes) but has no pricing math on top of that grammar.
- Fincept's global coverage (68 countries) versus Vyapaar's current India-only, 5-broker-adapter-driven market data (`vyapaar-market-angelone`, `-dhan`, `-fyers`, `-upstox`, `-zerodha`) is a scope question, not a mechanical gap: closing it would mean onboarding non-Indian venues and instrument conventions, which is a materially larger decision than adding indicators or a pricing crate.
- Separately, an open verification question exists (not yet checked this session): whether `vyapaar-market-angelone`/`-dhan`/`-fyers`/`-upstox`/`-zerodha` already use a table-driven mapping of broker-specific wire enums to canonical enums, or still rely on ad-hoc if-chains for broker-specific format translation. This question is cross-referenced from `vyapaar-exec/docs/plans/0001-crypto-execution-and-broker-wire-mapping.md`, which raises the same question for the execution-side broker crates in `vyapaar-exec`, since the two should be checked together rather than separately.

---

## 2. Component Specifications

### 2.1 Indicator breadth (`vyapaar-market-indicators`)

Add indicators following the exact existing pattern: one file per indicator (mirroring e.g. `rsi.rs`, `macd.rs`), each implementing the crate's incremental-update style so streaming tick/bar updates remain O(1) and WASM-portable. Prioritized candidates for the next batch (commonly requested, not an arbitrary "add more"):

1. Ichimoku Cloud (`ichimoku.rs`)
2. Keltner Channels (`keltner_channels.rs`)
3. Parabolic SAR (`parabolic_sar.rs`)
4. Commodity Channel Index — CCI (`cci.rs`)
5. Money Flow Index — MFI (`mfi.rs`)
6. Rate of Change — ROC (`roc.rs`)
7. TRIX (`trix.rs`)
8. Aroon Up/Down (`aroon.rs`)
9. Chaikin Money Flow (`chaikin_money_flow.rs`)
10. Chande Momentum Oscillator (`cmo.rs`)
11. Ultimate Oscillator (`ultimate_oscillator.rs`)
12. Elder Ray Index (`elder_ray.rs`)
13. Know Sure Thing — KST (`kst.rs`)
14. Vortex Indicator (`vortex.rs`)
15. Fisher Transform (`fisher_transform.rs`)
16. Pivot Points (standard/Fibonacci/Woodie, beyond existing CPR) (`pivot_points.rs`)
17. Hull Moving Average (`hull_ma.rs`)
18. Mass Index (`mass_index.rs`)

Each addition must follow the crate's existing incremental-update API shape and remain `#[cfg(test)]`-free in `src/` per workspace TDD rules (tests live in `vyapaar-market-indicators/tests/`, not inline).

### 2.2 Derivatives pricing (`vyapaar-market-pricing`, new crate)

A new crate, `vyapaar-market-pricing`, depending on `vyapaar-core` (for `Price`, `Quantity`, `Money` value objects) and `vyapaar-market-finance` (for existing F&O contract grammar types — expiry, lot size, strike rules). Component shape:

- A `PricingModel` trait, with an initial Black-Scholes implementation only. Do not attempt the full Fincept stack (14+ exotic option types, SABR/Heston calibration) in this crate's first slice.
- Outputs: Greeks — delta, gamma, theta, vega, rho — surfaced as `Decimal`-typed values at the API boundary, consistent with the workspace's zero-floats policy (§2.3 of the workspace `AGENTS.md`).
- **Explicit flag**: options pricing math (Black-Scholes closed form, implied-vol solvers via Newton-Raphson/Brent) is conventionally implemented in floating point for numerical stability and solver convergence. This crate needs a documented approach for reconciling that with the workspace's zero-floats policy. One plausible approach — not resolved here, to be decided during implementation — is computing the numerically intensive parts (Black-Scholes closed-form evaluation, implied-vol root-finding) internally in `f64`, then rounding to `Decimal` only at the crate's public API boundary, with the internal float usage documented and isolated from any domain aggregate that crosses `vyapaar-ledger`/`vyapaar-risk` boundaries.

### 2.3 Global market-data scope (decision point, no code)

State explicitly: expanding `vyapaar-market` beyond India-only, 5-broker coverage to global/68-country coverage (matching Fincept) is a scope decision requiring deliberate sign-off, not a default extension of the indicator/pricing work above. No resolution proposed in this document.

### 2.4 Open verification question (broker wire mapping)

Check whether `vyapaar-market-angelone`, `vyapaar-market-dhan`, `vyapaar-market-fyers`, `vyapaar-market-upstox`, and `vyapaar-market-zerodha` already use a table-driven data structure (analogous to a `BrokerEnumMap<T>` pattern seen in a different open-source trading terminal product, where it replaced per-broker if-chains across 14 of 16 brokers) to map broker-specific wire enums to canonical enums, or whether they still use ad-hoc if-chains. This is an open question, not a known gap — it has not been verified this session. If ad-hoc if-chains are found, a table-driven refactor would reduce duplication across the 5 broker crates. This should be checked jointly with the equivalent question for `vyapaar-exec`'s broker crates (see `vyapaar-exec/docs/plans/0001-crypto-execution-and-broker-wire-mapping.md`, Phase 1).

---

## 3. Implementation Phases

### Phase 1: Indicator breadth
Implement the prioritized indicator list in §2.1, one file per indicator, with parallel tests in `vyapaar-market-indicators/tests/`.

### Phase 2: `vyapaar-market-pricing` Black-Scholes slice
Stand up the new crate with the `PricingModel` trait and a Black-Scholes implementation producing `Decimal`-typed Greeks, resolving the float-internal/Decimal-boundary question documented in §2.2 as part of implementation.

### Phase 3: Broker wire-mapping verification/refactor
Verify the open question in §2.4 against the 5 broker crates; if ad-hoc if-chains are confirmed, scope and execute a table-driven refactor, coordinated with the parallel verification in `vyapaar-exec`.

### Phase 4: Global market-data scope decision
Explicitly decide whether to pursue global/68-country market-data coverage — decide, do not just default to yes.
