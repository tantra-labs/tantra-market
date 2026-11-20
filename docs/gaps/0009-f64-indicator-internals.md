# 0009 — f64 indicator internals (Zero-Floats debt)

**Status**: open

## What's stubbed or hardcoded

`crates/tantra-market-indicators/src/atr.rs` computes in `f64` (`prev_close: Option<f64>`, `seed_trs: Vec<f64>`,
`atr: Option<f64>`, `update_hlc(high: f64, low: f64, close: f64) -> Option<f64>`) although ATR feeds price-denominated
levels. Other indicators are presumed `f64`-internal and tagged `numeric: f64-legacy` in the library manifests;
audit list to be completed in T22.1.

| File | Status |
|---|---|
| `atr.rs` | **blocking** — must be Decimal before `ind.predictive-ranges` ships (root plan 0016 §4.3) |
| `cmo.rs` | f64-legacy candidate (VIDYA dependency; decide in plan 0005 T22.2) |
| `adx.rs, rsi.rs, stochastic.rs, cci.rs, bollinger.rs, keltner_channels.rs, supertrend.rs, parabolic_sar.rs, mfi.rs, ...` | f64-legacy candidates — audit list to be completed in T22.1 |

## Why

Root `.agents/rules/interface-first-and-modularity.md` §2.3 forbids floats in price/level math; ATR-derived bands
are price levels. Root plan 0016 §4.3 requires migration before Predictive Ranges.

## How to close it

1. Plan [0005](../plans/0005-concept-indicator-library.md) §2.6: migrate `Atr` to `Decimal` with an old-vs-new golden parity test.
2. T22.1 audit: grep `f64`/`f32` in `crates/tantra-market-indicators/src`, classify each file (price-level vs unitless oscillator), complete the table above.
3. Migrate price-level indicators first, then oscillators; flip each manifest `numeric: f64-legacy` -> `decimal`; close when none remain.
