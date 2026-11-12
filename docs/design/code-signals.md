# `vyapaar-market-signals` — Explainable Signal Detectors Reference

Documents `crates/vyapaar-market-signals/src/` (ADR 0031): a causal,
confluence-scored signal engine over a closed bar series. Pure — no clock, no
I/O, no lookahead.

## Module Map

| Module | Purpose |
|--------|---------|
| `catalog.rs` | Frozen reason ontology: `ReasonGroup`, `ReasonPolarity`, `SignalHorizon`, `ReasonCode`, `REASON_CODES` (30 codes), `DETECTORS` (13 specs), `RULESETS` (2 horizons), `Params` |
| `detector.rs` | `Detector` trait + `Firing` (one firing at one bar, with cited numbers) |
| `detectors/` | 11 detector families: `EmaCross`, `RsiTurn`, `MacdCross`, `RelativeVolume`, `Candlestick`, `BreakOfStructure`, `FractalPivot`, `Levels`, `VwapCross`, `CprBreakout`, `OrbBreakout` |
| `series.rs` | `Series` — indicator columns (`ema_fast/slow`, `rsi`, `macd`, `vol_sma`, `vwap`, `cpr`, `atr`) computed once per evaluation from `vyapaar-market-indicators` |
| `engine.rs` | `Signal`, `Reason`, `SignalDirection`, `UnixNanos`, `compute`, `signals_for_horizon`, trade setup (ATR stop + 1R/2R targets) |

## The Frozen Reason Catalogue

Every reason code, group, polarity and weight lives in `catalog.rs` in one
auditable table. A detector decides only *when* a `ReasonCode` fires and cites
its numbers; the engine reads group/polarity/weight from the table. Adding a
code is additive; renaming or reusing one is not.

30 codes span trend (`ema_cross_*`), momentum (`rsi_*_turn`, `macd_cross_*`),
volume (`rel_volume_spike_*`, `volume_surge_*`), price action
(`bullish_engulfing`, `bearish_engulfing`, `hammer`, `shooting_star`,
`break_of_structure_*`), levels (`break_prior_day_high/low`,
`break_52w_high/low`, `pivot_high/low`), VWAP (`vwap_cross_*`), and CPR/ORB
breakouts.

## Rulesets (Horizons)

| Ruleset | Horizon | `min_strength` | `cooldown_bars` |
|---------|---------|----------------|-----------------|
| `signals/day:v1` | `Intraday` | 0.40 | 3 |
| `signals/swing:v1` | `Swing` | 0.40 | 0 |

Both enable the same detector set (`ema_cross`, `rsi_turn`, `macd_cross`,
`relative_volume`, `candlestick`, `break_of_structure`, `pivot`, `levels`);
`Params::for_horizon` differs the numbers (intraday: 9/21 EMAs, RSI 14/30/70;
swing: 20/50 EMAs).

## The Engine (`engine.rs`)

`compute(bars, instrument, timeframe, ruleset, params)`:

1. Builds the shared `Series` from the indicator library.
2. Runs every enabled `Detector` over the series, collecting `Firing`s per bar.
3. Weighs bull vs bear by polarity/weight; a net-zero bar produces nothing.
4. Applies `min_strength` and per-side `cooldown_bars` gates.
5. Emits a `Signal` with `direction`, `strength`, `reasons`, sorted tags, and
   an ATR-derived trade setup (`stop_loss` + 1R/2R `targets`).

Causality: the signal anchored at bar *N* reads only bars `<= N`, so
truncating future bars never changes an earlier signal.

## Zero Floats Policy

Signal scoring is `f64` internally (weights are floats), but every price that
escapes the crate — `stop_loss`, `targets` — is an exact `vyapaar_core::Price`
(`Decimal`), converted at the boundary.

## Related

* `code-indicators.md` — the indicator library that powers `Series`.
* `vyapaar-core` — `Bar`, `InstrumentId`, `Price`.
