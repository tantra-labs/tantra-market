# 0005 — Concept & Indicator Library (`tantra-market-library`)

**Status**: Deferred (2026-11-20) — optional Rust port only. Parent: root plan 0016.

> **Superseded/Deferred**: the concept & indicator library is implemented in Python in `tantra-strategies`
> plan 0005 (`tantra_library` + `tantra-library-mcp`). This plan is now only an OPTIONAL Rust port of hot
> indicators (Predictive Ranges, VIDYA) if profiling shows Python is too slow. The content below is kept for
> that purpose and is deferred.

## 0. Executive Summary & Objective

Build one typed, versioned, self-describing registry of concepts, indicators, composites and detectors
(root plan 0016 §1–§3). A new L2 crate `tantra-market-library` embeds manifests (front-matter + Markdown),
validates the dependency DAG, builds any computable entry by `id + params`, and batch-evaluates it over bars
into an `IndicatorFrame`. It subsumes `render::CATALOG` and is the Layer-3 `eval_indicators` of root plan 0015
(no second evaluator). Descriptor DTOs (`EntryDescriptor`, `ParamSpec`, `OutputSpec`, `EntryKind`,
`Determinism`) live in `tantra-core::library` (additive, schemars) per root plan 0016 §3 decision 1; the core
change is tracked in `tantra-core` (to confirm in T22.1) and is not specified here.

## 1. Context & Gaps Covered

- `Indicator` (`tantra-market-indicators/src/indicator.rs`) has a GAT `Input<'a>` and is not object-safe.
- `render.rs` exposes `CATALOG: [&str; 8]` (names only); `tantra-market-signals::catalog::DETECTORS` lists 8
  `DetectorSpec { name, codes }` unlinked to indicators.
- `atr.rs` is `f64` internally (`prev_close`, `seed_trs`, `atr`; `update_hlc(f64,f64,f64) -> Option<f64>`),
  violating Zero-Floats and blocking price-level composites. Tracked in gap 0009 (filed with this plan).
- Submodules `momentum/ trend/ volatility/ volume/ india/ gpu/` are 1-line stubs; ~38 flat files hold the math.

## 2. Component Specifications & Phases

### 2.1 Crate layout (T22.1)

```text
crates/tantra-market-library/
  Cargo.toml            # deps: tantra-core, tantra-market-indicators, tantra-market-signals, serde, serde_yaml(or toml), thiserror
  src/lib.rs            # re-exports; no I/O, no clocks
  src/manifest.rs       # parse front-matter -> tantra_core::library::EntryDescriptor + body: &'static str
  src/registry.rs       # Registry { entries: BTreeMap<EntryId, Entry> }
  src/dag.rs            # depends_on graph, topo order, cycle + depth check
  src/dynamic.rs        # DynIndicator adapters, factory by id
  src/frame.rs          # IndicatorFrame, Column
  src/error.rs          # LibraryError
  src/embed.rs          # include_str! table generated in build.rs (sorted, deterministic)
  library/{families,concepts,primitives,indicators,composites,detectors,recipes}/*.md
  tests/                # golden/<id>.json, registry.rs, dag.rs, frame.rs, proptest_*.rs
```

Facade: re-exported from the `tantra-market` facade crate (root plan 0016 §3 decision 2). Crate must stay
`wasm32-unknown-unknown`-compatible (no `std::fs`, no threads; `build.rs` only generates the embed table).

### 2.2 Manifest format and embedding

One `.md` per entry: YAML front-matter holding the fields of root plan 0016 §1.1 (`id, kind, version, family,
concepts, aliases, depends_on, inputs, params, outputs, warmup_bars, determinism, numeric, render, provenance,
related`) followed by the Markdown body (What it is · Formula · How it's used · vs neighbours · Pitfalls · FAQ).
`build.rs` emits `static EMBEDDED: &[(&str /*path*/, &str /*include_str!*/)]`; `Registry::builtin()` parses all
at startup (or lazily via `OnceLock`) and returns `Result<Registry, LibraryError>`.

```rust
pub struct Registry { /* BTreeMap<EntryId, Entry> */ }
impl Registry {
    pub fn builtin() -> Result<&'static Registry, LibraryError>;
    pub fn get(&self, id: &str) -> Option<&Entry>;
    pub fn list(&self, kind: Option<EntryKind>, family: Option<&str>) -> Vec<&EntryDescriptor>;
    pub fn search(&self, q: &str) -> Vec<&EntryDescriptor>;   // id, aliases, concepts, body terms
    pub fn dependencies(&self, id: &str) -> Result<Vec<EntryId>, LibraryError>; // topo order
}
pub fn evaluate(reg: &Registry, id: &str, params: &ParamValues, bars: &[Bar])
    -> Result<IndicatorFrame, LibraryError>;
```

Build-time validation (a `tests/registry.rs` test and a `Registry::builtin()` check): unique ids; kind-prefix
matches `kind`; every `depends_on`/`related`/`concepts` target exists; DAG acyclic; max depth <= 8;
param `default` within `[min,max]`; every `computable` entry has a factory; every non-`family`/`concept`
entry has a non-empty body; `numeric: f64-legacy` entries are flagged and count-checked against gap 0009.

### 2.3 Dynamic evaluation

```rust
pub trait DynIndicator: Send {
    fn update_bar(&mut self, bar: &Bar) -> Option<OutputRow>; // OutputRow = SmallVec<[Cell; N]> aligned to OutputSpec
    fn reset(&mut self);
    fn is_ready(&self) -> bool;
}
pub struct IndicatorFrame { pub ts: Vec<UnixNanos>, pub columns: Vec<Column> }
pub struct Column { pub name: String, pub unit: Unit, pub values: ColumnData } // Decimal | State(i8) | Event(bool)
```

Each wrapped `Indicator` gets a thin adapter (generic over its `Input` conversion from `Bar`). Composites hold
child `Box<dyn DynIndicator>` built from their `depends_on` entries, so reuse is by id only (root plan 0016 §1
rules). `evaluate` enforces warm-up (`None` rows before `warmup_bars`), closed-bar semantics, `volume_quality`
checks, and max-bars caps supplied by the caller. Frames map to Arrow `Decimal128(38,8)` in `tantra-ffi`
(consumer concern).

### 2.4 `LibraryError`

```rust
#[derive(thiserror::Error, Debug)]
pub enum LibraryError {
    UnknownEntry(String), UnknownParam { entry: String, param: String },
    OutOfBounds { entry: String, param: String, detail: String },
    InvalidEnum { entry: String, param: String, value: String },
    InputQuality { entry: String, required: VolumeQuality, found: VolumeQuality },
    Cycle(Vec<String>), DepthExceeded(usize), NotComputable(String),
    InsufficientBars { needed: usize, got: usize }, Manifest { path: String, reason: String },
}
```

Closed set; no `String` errors across the port (root plan 0016 §4.4). `InputQuality` is returned for
`volume_quality: real` entries on tick-volume instruments (root plan 0016 §4.2).

### 2.5 Primitive extraction (T22.1/T22.2)

Extract, with behaviour-preserving goldens first: `prim.true-range` (from `Atr`), `prim.highest/lowest`
(rolling, from `donchian.rs`), `prim.median-price`, `prim.typical-price`, `prim.up-down-volume`, expose
`prim.swing-pivot` as a series from the `FractalPivot` logic; `prim.linreg` is deferred to the
`ind.predictive-channels` follow-up. Existing `Atr`/`Donchian` are reimplemented on the primitives.

### 2.6 ATR Decimal migration (blocks `ind.predictive-ranges`)

Change `Atr` to `rust_decimal::Decimal` fields (`prev_close: Option<Decimal>`, `seed_trs: Vec<Decimal>`,
`atr: Option<Decimal>`) and `update_hlc(&mut self, high: Decimal, low: Decimal, close: Decimal) -> Option<Decimal>`.
Wilder RMA stays `(prev*(n-1)+tr)/n` in Decimal; rounding policy fixed (no implicit quantisation; document
scale). Callers of the f64 signature are listed in T22.1 (to confirm) and migrated in the same change; a
parity test compares the old f64 goldens within 1e-9 on the fixed fixture before the f64 path is deleted.
Closes the `atr.rs` row of gap 0009.

### 2.7 New indicators (T22.2) — Decimal-native, `numeric: decimal`

- `ind.vidya` — Chande VIDYA: `alpha = 2/(n+1)`, `k = |CMO(m)|/100`, `v = alpha*k*src + (1-alpha*k)*v_prev`; builds on `ind.cmo`
  (cmo.rs is f64 -> listed in gap 0009; VIDYA either migrates CMO or uses a Decimal sibling — decision in T22.2).
- `ind.volume-delta-pct` — `(sum(up_vol) - sum(down_vol)) / sum(vol)` over window `n`, in `[-1,1]` (shown as %);
  `volume_quality: real`.
- `ind.alligator` — SMMA(hl2) jaw/teeth/lips with periods 13/8/5 and forward shifts 8/5/3 (params), shift
  implemented as output delay metadata, not lookahead.
- `ind.atr-bands` — `src ± mult*ATR`; thin wrapper.

### 2.8 Composites (T22.2)

**`ind.predictive-ranges`** (`depends_on: [ind.atr]`; params `length=200, factor=6, source, timeframe?`):

```text
a      = ATR(length) * factor          # recomputed each bar (Decimal)
if |src - center| > a:  center += sign(src - center) * a ; hold = a / 2 ; step = sign      # step event
else:                   step = 0                                                          # hold frozen at last step value
outputs: r2 = center + 2*hold, r1 = center + hold, center, s1 = center - hold, s2 = center - 2*hold, center_step in {-1,0,1}
```
Init: `center = src` and `hold = a/2` on the first ready ATR bar. `hold` is frozen between steps (it does NOT
track the live ATR) — this is the property the proptest asserts. A `timeframe` HTF variant reads only the last
**closed** HTF bar (`determinism: htf-repaints-until-htf-close` for live; backtests use closed-only), and a
`prev_*` view exposes the range active *before* the current bar so breakout detectors can pin the trigger
level (root plan 0016 §5). Exact initial-bar and equality (`>` vs `>=`) behaviour confirmed against the
published math in T22.2 before freezing goldens.

**`ind.vidya-trend`** — VIDYA ± `k*ATR` bands; trend flips when close crosses the opposite band; outputs
`trend in {-1,1}`, `upper`, `lower`, per-trend `volume_delta_pct`. Exact rules confirmed in T22.2 (root plan 0016 §7.1).

**`ind.reversal-bars`** — bullish: new N-bar low, close in upper half of the bar, bar below Alligator; mirror
for bearish. Exact rules confirmed in T22.2 (root plan 0016 §7.1).

### 2.9 Detectors (T22.2) — emit `tantra-market-signals::Signal`

New: `det.pr-center-step`, `det.pr-trend-regime` (param `k`, last K steps all up/down), `det.pr-zone-touch`
(`side: support|resistance`; low <= S1 / high >= R1), `det.pr-zone-close-break` (close beyond R1/S1 without a
center step), `det.reversal-bar`, `det.vidya-trend-flip` (root plan 0016 §2.4). Existing 8 detectors
(`Candlestick, BreakOfStructure, FractalPivot, RsiTurn, MacdCross, CprBreakout, VwapCross, OrbBreakout`) get
manifests with `depends_on`; `DETECTORS` becomes a view generated from the registry (`DetectorSpec` retained
for source compatibility for one release). Detector `params` use the same `ParamSpec`; events carry
`Reason` + the pinned trigger level in `Signal` metadata (field addition to confirm in T22.2).

### 2.10 Render-contract merge

`render::Presentation` (`Pane, PlotKind, ColorRole, SignalKind, Plot, Zone`) becomes the manifest `render`
block; `CATALOG` becomes `registry.list_with_render()`. `presentation(name)` and `validate(...)` stay as
thin shims for the existing 8 names until the UI migrates (T22.5).

### 2.11 Concept pages (own prose; clean-room)

First batch (root plan 0016 §2.6): `concept.atr`, `concept.volatility-range` (ATR bands / Keltner / Predictive
Ranges / Supertrend compared), `concept.adaptive-ma`, `concept.volume-delta`, `concept.reversal-bar`,
`concept.support-resistance`, `concept.swing-structure`, `concept.trend-pullback`, `concept.breakout`,
`concept.r-multiple`. Plus 10 indicator manifests in T22.1 (`atr, rma, sma, ema, rsi, macd, cmo, donchian, keltner-channels, supertrend`),
remainder wrapped in T22.2. Each page ends with auto-listed implementations. LuxAlgo/TradingView text is never
copied; `provenance` records origin; licence review is a T22.1 exit gate.

## 3. Tests & Verification

- Tests live only in `crates/tantra-market-library/tests/` (no inline `#[cfg(test)]`).
- **Golden vectors** `tests/golden/<id>.json` (fixed bars -> exact Decimal columns); Predictive Ranges goldens
  cross-checked once against TradingView on a public symbol.
- **Proptest**: PR center moves only when `|src-center| > a`; `hold` constant between steps; `r2>r1>center>s1>s2`;
  `center_step=0` implies unchanged center; VIDYA bounded by min/max of source window; warm-up bars yield `None`;
  evaluate is deterministic and equal to bar-by-bar `update_bar`.
- **Registry**: DAG/cycle/depth/unknown-param/out-of-bounds/enum/InputQuality negative tests; manifest lints from §2.2.
- **Parity**: Decimal ATR vs old f64 goldens (tolerance) before deleting the f64 path.
- Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, `make test`, `make prod`;
  `cargo check -p tantra-market-library -p tantra-market-indicators --target wasm32-unknown-unknown`.

## 4. Open Questions

1. Front-matter format: YAML vs TOML (YAML proposed; `serde_yaml` wasm-clean — confirm in T22.1).
2. Does CMO migrate to Decimal as part of VIDYA, or does VIDYA ship on a Decimal CMO sibling? (T22.2)
3. `Signal` metadata field for pinned trigger level — additive change to `tantra-market-signals::engine`.
4. Whether recipes' manifests live in this crate (proposed, root plan 0016 §7.2); execution stays in `tantra-strategies`.
5. Licence outcome for LuxAlgo-derived ideas (root plan 0016 §7.4) gates T22.2 merges.
