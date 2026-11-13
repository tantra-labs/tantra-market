# Gap 0005 — No Elephant-Zone / OI-Level / Gamma / Order-Block Indicators

**Status**: open — tracked by root master plan [`docs/plans/0005-elephant-edge-strategy-platform.md`](../../../docs/plans/0005-elephant-edge-strategy-platform.md)

## What's Missing

The chain reader (gap 0001, closed) and OI stores + buildup (gap 0002, closed)
exist, but **no indicator layer turns bars + OI into the level feeds** the
Elephant Edge (EE+) strategy consumes:

1. **Elephant percentile zones** — the core EE+ level family: historical
   intraday session ranges → percentile distances from the session open →
   two support / two resistance **bands** (defaults 25/35 and 63/74
   percentiles), session-fixed, volatility-adaptive. No indicator implements
   session-range percentile projection.
2. **OI-wall / gamma levels** — computing support/resistance strikes from the
   closed `OpenInterestStore` (top-2 CE/PE OI walls, building-ΔOI scoring) and
   a **per-strike gamma-exposure / gamma-flip** profile (Black-76 gamma × OI ×
   lot × spot scaling). Raw OI rows exist; the derived level/GEX math does not.
3. **Order-Block detection** — last opposing candle before a displacement move
   (displacement = candle range ≥ `displacement_atr_mult` × ATR), producing
   bullish/bearish OB zones from 1-minute bars.

## Why

`tantra-strategies` plan 0001 consumes these as **streaming indicators** (same
pattern as the existing `IndicatorRegistry`), so the math must live in the
market-data/indicator bounded context — not duplicated in the Python strategy —
to keep research-live parity and Decimal semantics in one place. The chain/OI
data plumbing is already done; this gap is the derived-level computation layer.

## How to Close It

1. **`ElephantZones` indicator** (new `tantra-market-indicators` module):
   - Input: session bars; output: `(s1, s2, r1, r2)` Decimal bands.
   - State: historical session ranges (rolling N sessions, default 20);
     percentile distances via exact ranking (no float interpolation) projected
     symmetrically from the current session open.
   - Streaming: recompute on each bar close; session-fixed (reset per day).
2. **`OiLevels` indicator**: reads the `OpenInterestStore` port → top-2 call
   walls above spot, top-2 put walls below spot, ΔOI building score per wall.
3. **`GammaProfile` indicator**: per-strike GEX from the chain snapshot +
   OI store → `GammaExposure { per_strike, net, flip }` (Decimal).
4. **`OrderBlock` indicator**: 1-min bar input → displacement detection →
   last opposing candle zone (body-range based), with a
   `displacement_atr_mult` parameter.
5. Tests in `tests/` per the Parallel Test Directory Rule: golden vectors for
   percentile math, GEX per strike, and OB zones; `proptest` invariants
   (bands ordered, flip within struck range, zones non-empty).
6. All outputs `Decimal`; zero floats; indicators compile to
   `wasm32-unknown-unknown` (portable compute).

## Verification

`cargo test --workspace --profile prod` green in `tantra-market`; the four
indicators produce the golden level outputs on seeded session bars + seeded OI
store; `make prod` green end-of-turn (build-discipline rule).
