# 0002 — Chart Pattern Analyzer (geometric + multi-bar candlestick patterns with trade geometry)

**Status**: Proposed

Roadmap: root [plan 0008](../../../docs/plans/0008-trade-signal-generator.md) §3 (pattern-driven
trade plans). Wire type: [`tantra-core` plan 0005](../../../tantra-core/docs/plans/README.md#implemented-plans) §2a (`ChartPattern`).

## 0. Executive Summary & Objective

Add a chart pattern analyzer to `tantra-market-signals`. For each pattern it detects, the analyzer reports exact `Decimal` **trade geometry**, not just a firing:

- **breakout level**: the entry trigger
- **invalidation level**: the structural stop
- **measured-move target**: the pattern height projected from the breakout (`Option<Price>`)
- **stage (`PatternStage`)**: `FormingArmed` (pre-breakout setup ready for a resting `StopAbove`/`StopBelow` entry) vs `BreakoutConfirmed` (completed by breakout close or multi-bar candle completion)
- **quality score** (`0..=100`, advisory until Phase D)

The `TradePlanner` (core plan 0005) can then build entry, SL and targets from price structure instead of ATR multiples alone, and reject setups whose measured move gives less than `min_reward_risk` before entry.

## 1. Context & Gaps Covered

**Existing in `crates/tantra-market-signals/src/`:**
- `Detector` trait (`detector.rs`) operating over `Series` (`series.rs`), which converts all bar OHLCV into `Vec<f64>` and drops `DateTime<Utc>` timestamps (`days: Vec<i64>` only).
- `Firing { code, bar, detail, evidence: Vec<(&str, f64)> }` and `engine::Signal` (local `f64`-weighted struct, distinct from `tantra_core::Signal`).
- `detectors/price_action.rs`: `Candlestick` (engulfing, hammer, shooting star), `BreakOfStructure`, `FractalPivot { span }` (confirmed causally at `i + span`).
- Other detectors in `catalog.rs` with frozen reason codes.

**Missing / Gaps to Address:**
- Every multi-bar **geometric** pattern and 2–3-bar candlestick pattern.
- Causal `PatternAnalyzer` API taking `&[Bar]` directly (preserving exact `Decimal` `Price` and `DateTime<Utc>` timestamps — `Detector::detect(&Series)` cannot produce zero-float `ChartPattern` prices or `DateTime<Utc>` without `&[Bar]`/`timestamps`!).
- Explicit distinction between `FormingArmed` setups and `BreakoutConfirmed` patterns.

**Who needs this:**
- Root [plan 0009](../../../docs/plans/0009-qullamaggie-swing-trading-platform.md) (Qullamaggie): flag, VCP and Darvas breakout detection.
- Root [plan 0010](../../../docs/plans/0010-screener-taxonomy-and-alerts.md) & [`tantra-data` plan 0008](../../../tantra-data/docs/plans/0008-screen-dsl-catalog-qvm.md): Candlestick and Pattern screen categories.
- `tantra-ui` gap [0007](../../../tantra-ui/docs/gaps/README.md): chart annotations from `ChartPattern` pivots/levels — **closed 2026-11-17** (T12.9 chart primitives; the Studio view consuming them remains a follow-on).

## 2. Component Specifications & Phases

### Phase A — Causal Swing Skeleton & `PatternAnalyzer` Contract
- Add `PatternAnalyzer::analyze(bars: &[Bar], timeframe: Timeframe, params: &PatternParams) -> Vec<ChartPattern>` (alongside optional `Detector` emission of `Firing`s for confluence scoring in `engine.rs`). Retaining `&[Bar]` avoids `Series`'s `f64` loss and provides `DateTime<Utc>` for `PatternPivot.bar_ts`, `formed_from`, `armed_at`, and `confirmed_at`.
- `ZigZag { min_swing_pct | min_swing_atr }` plus a causal pivot tracker (`FractalPivot { span }` confirmed strictly at `bar + span`).
- **No look-ahead across `FormingArmed` vs `BreakoutConfirmed`**:
  - A `FormingArmed` pattern may only use pivots whose confirmation bar (`pivot_bar + span`) is `<= armed_at`. Once the breakout threshold and invalidation are known, it emits `FormingArmed` so `TradePlanner` can stage `EntryRule::PatternBreakout` (`StopAbove`/`StopBelow`).
  - A `BreakoutConfirmed` pattern anchors at the breakout close bar (`confirmed_at >= armed_at`); a pattern must **never** be backdated to an unconfirmed pivot or depend on post-breakout bars (`i + span > breakout_bar`) to justify an earlier entry.

### Phase B — Geometric pattern detectors

| Pattern | Breakout / entry | Invalidation (SL) | Measured target | Default stage |
|---|---|---|---|---|
| Bull/bear flag, pennant | upper/lower flag line | flag low/high | flagpole height from breakout | `FormingArmed` / `BreakoutConfirmed` |
| Ascending/descending/symmetric triangle | flat/converging boundary | last confirmed swing low/high inside triangle | widest-part height | `FormingArmed` / `BreakoutConfirmed` |
| Rectangle / range, Darvas box | box top/bottom | box opposite side (or mid, configurable) | box height | `FormingArmed` / `BreakoutConfirmed` |
| Double top/bottom | neckline | second peak/trough | peak-to-neckline height | `BreakoutConfirmed` |
| Head & shoulders (and inverse) | neckline | right shoulder | head-to-neckline height | `BreakoutConfirmed` |
| Rising/falling wedge | wedge boundary (reversal direction) | wedge extreme | wedge base height | `FormingArmed` / `BreakoutConfirmed` |
| Cup-and-handle | handle high | handle low | cup depth | `FormingArmed` |
| VCP (volatility contraction) | last contraction high (pivot) | last contraction low | `None` (trail-only / ATR ladder fallback in `TradePlan`) | `FormingArmed` |

Shared rules:
- Tolerances such as near-equal peaks, line-fit error and minimum bars are `Decimal` / integer spec parameters with documented defaults.
- **Quality score** (`0..=100`): tightness / line-fit error, volume dry-up inside contraction, breakout relative volume (for `BreakoutConfirmed`), prior trend strength, pattern duration. Advisory only until Phase D validates it.
- Invalidated patterns (price crosses `invalidation_level` before breakout) transition out and are never emitted as active `FormingArmed` setups after invalidation.

### Phase C — Multi-bar candlesticks
- Morning star / evening star, bullish/bearish harami, piercing line / dark cloud cover, three white soldiers / three black crows, inside bar and outside bar (all emit `BreakoutConfirmed` with `measured_target: None` or bar-range projection as configured).
- New frozen reason codes appended to `catalog.rs` (additive only; existing codes never renamed); `tantra-data` plan `0008` Phase C Candlestick/Darvas screen presets reference these exact catalog codes (via FFI/bridge rather than duplicating logic in DuckDB SQL).

### Phase D — Per-pattern edge statistics
- Offline point-in-time study over the lakehouse measuring per `PatternKind × Timeframe`: hit rate to target, failure rate (invalidation), average and median `R`, and bars to target.
- Published as a versioned data table consumed by `TradePlanner` presets; only then may `quality` and edge stats scale `risk_pct` or gate entries.

### Output type and number handling

Returns `tantra_core::ChartPattern` (core plan 0005 §2a):
`ChartPattern { kind, direction, timeframe, stage, pivots, breakout_level, invalidation_level, measured_target?, quality, formed_from, armed_at, confirmed_at? }`.

**Zero-floats rule for trade geometry:**
- `Series`/`Firing.evidence` remain `f64` for legacy display evidence, **but every `Price` in `ChartPattern` must be `Decimal` derived directly from `&[Bar]`**.
- Pivot prices come verbatim from `bars[idx].high` / `bars[idx].low` (`Price`).
- Boundary-line extrapolations and measured targets use `Decimal` arithmetic (`checked_*` / `round_dp`), never `f64` conversions. (`f64` is allowed only in heuristic pivot-selection scoring if never propagated into prices.)

## 3. Test Plan (TDD)

All tests go in `crates/tantra-market-signals/tests/`:
- `chart_patterns_unit.rs`: hand-built `Bar` fixtures per pattern asserting exact `Decimal` breakout, invalidation, target, `stage`, and `confirmed_at`.
- `chart_patterns_causal.rs`: proptest verifying that truncating `bars` after index `i` never changes any `ChartPattern` with `armed_at <= bars[i].timestamp` or `confirmed_at <= Some(bars[i].timestamp)` (strict no-repaint).
- `chart_patterns_golden.rs`: golden JSON of patterns detected on frozen NSE bar extracts.
- Negative tests:
  - near-miss shapes that must not fire
  - patterns invalidated before breakout must not emit
  - `FractalPivot` lag test proving no pivot from `j + span > i` is used when building a pattern at bar `i`.

## 4. Dependencies & Tier Notes

- L2 crate. Depends on L0 `tantra-core` (for `Bar`, `Timeframe`, `ChartPattern`) and L2 `tantra-market-indicators` only.
- `TradePlanner` stays in L0 core and receives `ChartPattern` through `PlannerInputs.pattern` (zero upward edges).
- Must stay WASM-compatible where `tantra-market-signals` already is.
- Consumers:
  - `tantra-runtime` signal scan and backtest (`tantra-runtime` plan `0006`)
  - `tantra-ffi` bindings (`tantra-ffi` plan `0004`)
  - `tantra-data` screen catalog Candlestick/Pattern presets (`tantra-data` plan `0008`)
  - agent `chart_patterns` tool (`tantra-agent` plan `0014`)
  - `tantra-ui` chart annotation primitives (`tantra-ui` gap `0007`)

## 5. Open Questions

- Swing threshold default: ATR-based (scale-free across instruments) by default, percentage as an option.
- Higher-timeframe evaluation: caller passes explicit `Timeframe` (`D1`, `W1`, `M5`) so `PatternAnalyzer` stays single-series pure while callers compose multi-timeframe scans.
