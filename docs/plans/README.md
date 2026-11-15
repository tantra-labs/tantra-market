# Plans Index — `tantra-market`

Per workspace rule `.agents/rules/gap-tracking.md`:
- Gaps are individual features (`docs/gaps/XXXX-*.md`).
- Plans are collections of gaps (`docs/plans/XXXX-*.md`).
- When implemented, plan documents are cleaned up from `docs/plans/` and recorded in this index.

## Active Plans

| # | Title | Target Scope | Status |
|---|-------|--------------|--------|
| [0002](0002-chart-pattern-analyzer.md) | Chart Pattern Analyzer — geometric patterns (flag, triangle, rectangle/Darvas, double top/bottom, H&S, wedge, cup-and-handle, VCP) + multi-bar candlesticks, emitting breakout/invalidation/measured-target geometry | `tantra-market-signals` | Proposed — root plan 0008 §3; wire type in `tantra-core` plan 0005 §2a |

## Implemented Plans

| # | Title | Target Scope | Implemented In |
|---|-------|--------------|----------------|
| 0001 | Technical Indicator Breadth, Market Data Scope, and Derivatives Pricing | `tantra-market-indicators`, `tantra-market-pricing` | Phases 1–3 implemented (`ddb5c3f`, `1983512`, `602a710`); Phase 4 decided — **decline global/68-country expansion by default; Tantra market data remains India-only (5 brokers)**. Plan document removed from active plans (cleanup commit). |
