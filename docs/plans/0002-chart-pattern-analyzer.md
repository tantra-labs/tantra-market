# 0002 — Chart Pattern Analyzer (geometric + multi-bar candlestick patterns with trade geometry)

**Status**: Proposed

Roadmap: root [plan 0008](../../../docs/plans/0008-trade-signal-generator.md) §3 (pattern-driven
trade plans). Wire type: [`tantra-core` plan 0005](../../../tantra-core/docs/plans/0005-trade-planner-and-manager.md) §2a (`ChartPattern`).

## 0. Executive Summary & Objective

Add a chart pattern analyzer to `tantra-market-signals`. For each pattern it detects, the analyzer reports the pattern's **geometry**, not just a firing:

- **breakout level**: the entry trigger
- **invalidation level**: the structural stop
- **measured-move target**: the pattern height projected from the breakout
- **quality score**

The `TradePlanner` (core plan 0005) can then build entry, SL and targets from the price structure instead of ATR multiples alone. It can also reject setups whose measured move gives less than the minimum reward:risk ratio before entry.

## 1. Context & Gaps Covered

**Existing in `crates/tantra-market-signals/src/`:**
- The `Detector` trait (`detector.rs`). `detect(&Series) -> Vec<Firing>` is **causal**: a firing at bar `i` reads only data at `<= i`.
- `Firing{code, bar, detail, evidence: Vec<(&str, f64)>}`.
- `detectors/price_action.rs`:
  - `Candlestick`: bullish/bearish engulfing, hammer, shooting star.
  - `BreakOfStructure`.
  - `FractalPivot{span}`: confirmed at `i+span`.
- Also `orb.rs`, `cpr_breakout.rs`, `levels.rs`, `trend.rs`, `momentum.rs`, `volume.rs`, `vwap_cross.rs`, with reason codes frozen in `catalog.rs`.

**Missing:**
- Every multi-bar **geometric** pattern.
- 2–3-bar candlestick patterns.
- Any output that carries pattern geometry.

**Who needs this:**
- Root [plan 0006](../../../docs/plans/0006-qullamaggie-swing-trading-platform.md) (Qullamaggie): its breakout setup needs flag, VCP and Darvas detection.
- Root [plan 0007](../../../docs/plans/0007-screener-taxonomy-and-alerts.md): its screener catalog has a Candlestick category (bullish/bearish × 1/2/3+-day).

## 2. Component Specifications & Phases

### Phase A — Swing skeleton
- `ZigZag{min_swing_pct | min_swing_atr}` plus a pivot sequence built on `FractalPivot`.
- Each pivot records `(bar, price, kind, confirmed_at)`. `confirmed_at` is the bar at which the pivot became knowable.
- Every pattern is anchored at its **confirmation bar**, never at the pivot bar. This keeps the analyzer free of repainting and look-ahead.

### Phase B — Geometric pattern detectors

| Pattern | Breakout / entry | Invalidation (SL) | Measured target |
|---|---|---|---|
| Bull/bear flag, pennant | upper/lower flag line | flag low/high | flagpole height from breakout |
| Ascending/descending/symmetric triangle | flat/converging boundary | last swing low/high inside triangle | widest-part height |
| Rectangle / range, Darvas box | box top/bottom | box opposite side (or mid, configurable) | box height |
| Double top/bottom | neckline | second peak/trough | peak-to-neckline height |
| Head & shoulders (and inverse) | neckline | right shoulder | head-to-neckline height |
| Rising/falling wedge | wedge boundary (reversal direction) | wedge extreme | wedge base height |
| Cup-and-handle | handle high | handle low | cup depth |
| VCP (volatility contraction) | last contraction high (pivot) | last contraction low | n/a: trail-only, no measured target (preset uses ladder) |

Shared rules:
- Tolerances such as near-equal peaks, line-fit error and minimum bars are spec parameters, each with a documented default.
- **Quality score** (0–100) is built from:
  - tightness / line-fit error
  - volume dry-up inside the pattern
  - breakout relative volume
  - prior trend strength
  - pattern duration
- The score is advisory. It must not affect sizing until Phase D validates it.

### Phase C — Multi-bar candlesticks
- Morning star / evening star, bullish/bearish harami, piercing line / dark cloud cover, three white soldiers / three black crows, inside bar and outside bar.
- New frozen reason codes are appended to `catalog.rs`; existing codes are never renamed.

### Phase D — Per-pattern edge statistics
- An offline study over the lakehouse, run point-in-time, measures for each pattern kind × timeframe:
  - hit rate to target
  - failure rate (invalidation)
  - average and median R
  - bars to target
- The stats are published as a versioned table: data only, consumed by `TradePlanner` presets.
- Only then may `quality` and edge stats scale risk or gate entries.

### Output type and number handling

Detectors return `tantra_core::ChartPattern` (core plan 0005 §2a) alongside the usual `Firing`:

`ChartPattern { kind, direction, pivots, breakout_level, invalidation_level, measured_target?, quality, formed_from, confirmed_at }`

**Zero-floats note:** `Series`/`Firing.evidence` are `f64` today, which is fine for display evidence. Every **price** in `ChartPattern` must be `Decimal`:
- Pivot prices are taken exactly from the source bars by index, not from floating-point fits.
- Boundary-line values at the breakout bar and measured targets are computed in `Decimal`.
- Line fitting may use `f64` internally only to *select* pivots, never to produce a price.

## 3. Test Plan (TDD)

All tests go in `crates/tantra-market-signals/tests/`:
- `chart_patterns_unit.rs`: one hand-built bar fixture per pattern, asserting the expected breakout, invalidation and target levels exactly.
- `chart_patterns_causal.rs`: a proptest that truncating the series after bar `i` never changes patterns confirmed at bars `<= i` (no repaint).
- `chart_patterns_golden.rs`: golden JSON of patterns detected on frozen NSE bar extracts.
- Negative tests:
  - near-miss shapes that must not fire
  - a pattern invalidated before breakout, which must not emit a pattern firing

## 4. Dependencies & Tier Notes

- L2 crate. Depends on L0 `tantra-core` only (for `ChartPattern`).
- `TradePlanner` stays in core and receives patterns through `PlannerInputs.pattern`, so there is no upward edge.
- Must stay wasm-compatible where `tantra-market-signals` already is.
- Consumers:
  - `tantra-runtime` signal scan and backtest (runtime plan 0006)
  - `tantra-ffi` bindings (ffi plan 0002)
  - `tantra-data` screen catalog Candlestick/Pattern presets (data plan 0008)
  - an agent `chart_patterns` tool (agent plan 0014)

## 5. Open Questions

- Should the swing size default to ATR-based (scale-free across instruments) or percentage-based? Proposal: ATR-based by default, percentage as an option.
- Should patterns be evaluated on higher timeframes from resampled daily bars (W/M), or only on native series?
