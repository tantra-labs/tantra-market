# `vyapaar-market-indicators` — Streaming Indicator Catalog Reference

Documents `crates/vyapaar-market-indicators/src/`: the shared `Indicator`
trait and its 32 implemented indicator families, plus the screener and render
contracts. Pure compute — no `tokio`, no I/O, WASM-portable.

## The `Indicator` Trait (`indicator.rs`)

```rust
pub trait Indicator {
    type Input<'a>;          // may borrow from the caller (e.g. &Bar)
    type Output: Copy;       // returned by value, no allocation
    fn update(&mut self, input: Self::Input<'_>) -> Option<Self::Output>;
    fn value(&self) -> Option<Self::Output>;
    fn reset(&mut self);
    fn is_ready(&self) -> bool { self.value().is_some() }
}
```

Incremental, single-pass: nothing buffers the full series; each indicator
holds only the state it needs, so it can run over an unbounded live stream.
`update` returns `None` until the indicator has primed. A GAT input lets ATR
etc. read `&Bar` without cloning.

## Indicator Catalog (32)

| Family | Type(s) |
|--------|---------|
| Moving averages | `Sma`, `Ema`, `Rma`, `HullMa`, `Vwap` |
| Oscillators | `Rsi`, `Stochastic`, `Cci`, `Cmo`, `Roc`, `Kst`, `Trix`, `Mfi`, `ChaikinMoneyFlow`, `MassIndex`, `UltimateOscillator`, `WilliamsR`, `FisherTransform` |
| Bands / channels | `BollingerBands`, `KeltnerChannels`, `Donchian`, `Atr`, `Supertrend`, `Vortex` |
| Trend / structure | `Adx`, `Aroon`, `Macd`, `Ichimoku`, `ParabolicSar`, `ElderRay`, `Cpr`, `PivotPoints` |
| Volume | `Obv` |

All 32 are re-exported from `lib.rs` together with the `Indicator` trait.
`bar_f64` / `quantity_f64` read exact `Price`/`Quantity` as the `f64` input
most indicators consume — the documented indicator-layer float boundary
(indicators are numeric transforms, not domain aggregates).

## Screener (`screener.rs`)

Predicate evaluation over a bar series: `FilterOp` (Gt/Gte/Lt/Lte/Eq/Ne/
Between/In/NotIn/Like/Has/CrossesAbove/CrossesBelow), `MetricRef` +
`MetricPeriod`, `ScreenerFilterPredicate`, plus a `Screener`-style evaluator
built on `Sma`/`Rsi` and canonical `vyapaar_core::Bar`.

## Render Contract (`render.rs`)

Presentation-only contract for charting: `catalog_names()` returns the 8
indicators the WASM surface computes (`sma`, `ema`, `rsi`, `macd`,
`bollinger`, `atr`, `williams_r`, `stochastic`); `presentation(name)` returns
the pane/plot/zone spec; `validate` checks a presentation against an
indicator's declared outputs. Nothing here is a wire message and nothing can
place an order.

## Stub Modules

`gpu/`, `india/`, `momentum/`, `trend/`, `volatility/`, `volume/` exist as
empty scaffolded modules (single doc comment, no code).

## Related

* `code-signals.md` — the signals engine builds its `Series` from these indicators.
* `vyapaar-core` — `Bar`, `Price`, `Quantity`.
