# 0007 — ADR%(20) and Chandelier Exit indicators missing

**Status**: open

## What's stubbed or hardcoded

`crates/tantra-market-indicators/src/` ships `atr.rs`, `supertrend.rs`, `parabolic_sar.rs`, `keltner_channels.rs`,
`donchian.rs`, `pivot_points.rs`, `cpr.rs`, `adx.rs` but **no ADR%(20)** (average daily range %) and **no Chandelier Exit**.
Callers of the trade planner currently have nowhere to source these.

## Why

Needed by the trade-signal-generator presets ([root plan 0008](../../../docs/plans/0008-trade-signal-generator.md)):
`qullamaggie` rejects entries whose stop distance exceeds 1 x ADR%(20) (root plan 0006, gap row 22); `atr_swing` and `ee_plus`
use a Chandelier trail on the runner. Core plan `tantra-core/docs/plans/0005-trade-planner-and-manager.md` takes both as caller-supplied
`PlannerInputs` / manager snapshot values.

## How to close it

1. Tests first in `crates/tantra-market-indicators/tests/`: golden vectors + conformance against a reference implementation.
2. `adr_pct.rs`: streaming `AdrPct { period }` = mean over N bars of (high/low - 1) x 100; `Decimal` only; warm-up returns `None`.
3. `chandelier.rs`: streaming `ChandelierExit { period, mult }` emitting long stop (highest high - mult x ATR) and short stop (lowest low + mult x ATR), reusing `atr.rs`.
4. Follow the existing indicator trait/registry conventions; export from the crate; wasm32-compatible, zero I/O/clock.
5. proptest: long chandelier <= highest high; ADR% >= 0; streaming == batch recompute.
6. Register in the indicator catalogue so `tantra-ffi` and the SDK `IndicatorRegistry` can expose them.
