# Plans Index — `tantra-market`

Per workspace rule `.agents/rules/gap-tracking.md`:
- Gaps are individual features (`docs/gaps/XXXX-*.md`).
- Plans are collections of gaps (`docs/plans/XXXX-*.md`).
- When implemented, plan documents are cleaned up from `docs/plans/` and recorded in this index.

## Active Plans

| # | Title | Target Scope | Status |
|---|-------|--------------|--------|
| [0002](0002-chart-pattern-analyzer.md) | Chart Pattern Analyzer — geometric patterns (flag, triangle, rectangle/Darvas, double top/bottom, H&S, wedge, cup-and-handle, VCP) + multi-bar candlesticks, emitting breakout/invalidation/measured-target geometry | `tantra-market-signals` | Proposed — root plan 0008 §3; wire type in `tantra-core` plan 0005 §2a |
| [0003](0003-experimental-analysis-period-and-markov-engine.md) | Regime-Aware Period Slicer & Statistical Markov Engine (`PeriodSchedule` + `tantra-market-finance::studies`) | `tantra-market-finance` | Proposed (2026-11-18) — Track **T20.2**, root plan [`0014`](../../../docs/plans/0014-experimental-analysis-statistical-workbench.md); depends on `tantra-data` plan `0010` |
| [0005](0005-concept-indicator-library.md) | Concept & Indicator Library (`tantra-market-library`: manifests, registry, DAG, `DynIndicator`/`IndicatorFrame`, ATR Decimal migration, Predictive Ranges, detectors) | `tantra-market-library`, `tantra-market-indicators`, `tantra-market-signals` | Proposed (2026-11-20) — root plan 0016 / T22.1–T22.2, design only |

## Implemented Plans

| # | Title | Target Scope | Implemented In |
|---|-------|--------------|----------------|
| 0001 | Technical Indicator Breadth, Market Data Scope, and Derivatives Pricing | `tantra-market-indicators`, `tantra-market-pricing` | Phases 1–3 implemented (`ddb5c3f`, `1983512`, `602a710`); Phase 4 decided — **decline global/68-country expansion by default; Tantra market data remains India-only (5 brokers)**. Plan document removed from active plans (cleanup commit). |
| 0004 | Composable Strategy, Optimization-Loop & Expiry-Hedge Primitives (`Track T21.1`) | `tantra-market-finance::studies::pipeline` | Implemented (2026-11-20) — Phase 1 of [`0015`](../../../docs/plans/0015-elephant-edge-outer-buffer-strategy-and-agent-workbench.md): `SessionClock` + `SplitArtifactGuard`, `ZoneExtractor` (`ElephantZonesExtractor`, `Orb15Extractor`), `IndicatorPrimitive` (`StandardIndicatorEvaluator`), `EntrySignalPrimitive` (`ZoneTouchEntryFilter`, `RejectionWickFilter`, `OrbBreakoutEntryFilter`), `StopExitPolicy` (`OuterBufferStopPolicy`), `HedgeOverlayPolicy` (`HedgeSynthesizer`), `StrategyLoopKernel`, and IS/OOS + WFE + DSR `ParameterSweepRunner` in pure `Decimal` (`wasm32-unknown-unknown` green, golden-vector parity verified in `tests/pipeline_unit.rs`). Plan document removed per `.agents/rules/gap-tracking.md`. |
