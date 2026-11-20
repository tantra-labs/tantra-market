# 0009 — f64 indicator internals (Zero-Floats debt)

**Status**: open

> **Note (2026-11-20)**: no longer blocking Track T22 — the library is implemented in Python (`tantra-strategies` plan 0005) and is Decimal-native. Kept open as general zero-floats debt in `tantra-market-indicators`.

## What's stubbed or hardcoded

`crates/tantra-market-indicators/src/atr.rs` computes in `f64` (`prev_close: Option<f64>`, `seed_trs: Vec<f64>`,
`atr: Option<f64>`, `update_hlc(high: f64, low: f64, close: f64) -> Option<f64>`) although ATR feeds price-denominated
levels. Other indicators are presumed `f64`-internal and tagged `numeric: f64-legacy` in the library manifests;
audit list to be completed in T22.1.

| File | Status |
|---|---|
| `atr.rs` | **migrated to Decimal 2026-11-20** (golden parity with `tantra_library`, `tests/golden_atr_cmo.rs`). `update_hlc_f64` is a presentation shim; keltner/supertrend/render/signals series/chat payloads still convert at the presentation boundary |
| `cmo.rs` | **migrated to Decimal 2026-11-20** (golden parity with `tantra_library`; `update_f64` shim) |
| `adx.rs, rsi.rs, stochastic.rs, cci.rs, bollinger.rs, keltner_channels.rs, supertrend.rs, parabolic_sar.rs, mfi.rs, ...` | f64-legacy candidates — audit list to be completed in T22.1 |

## Why

Root `.agents/rules/interface-first-and-modularity.md` §2.3 forbids floats in price/level math; ATR-derived bands
are price levels. Root plan 0016 §4.3 requires migration before Predictive Ranges.

## How to close it

1. Plan [0005](../plans/0005-concept-indicator-library.md) §2.6: migrate `Atr` to `Decimal` with an old-vs-new golden parity test.
2. T22.1 audit: grep `f64`/`f32` in `crates/tantra-market-indicators/src`, classify each file (price-level vs unitless oscillator), complete the table above.
3. Migrate price-level indicators first, then oscillators; flip each manifest `numeric: f64-legacy` -> `decimal`; close when none remain.
