# `vyapaar-market` — Migration Map (`honba-me` → `vyapaar-market`)

> **Source**: `honba-workspace/honba-me` (Tier 2A)
> **Target**: `vyapaar-market`
> **Status**: Migration complete. Workspace is now **14 crates**; `dhan` and
> `pricing` were added after the original 12-crate plan.

## Sub-Crate Mapping Table

| Source Crate (`honba-me/crates/`) | Target Crate (`vyapaar-market/crates/`) | Notes |
| :--- | :--- | :--- |
| `honba-me-core` | `vyapaar-market-core` | `lib.rs` (feed ports, mapper, coalescing, reconnection, enricher) + `ws_client.rs`; tests extracted to `tests/`. |
| `honba-me-market` | `vyapaar-market-finance` | Generic market contracts + `india`/`null` packs. |
| `honba-me-normalizer` | `vyapaar-market-normalizer` | Instrument master + corporate actions. |
| `honba-me-indicators` | `vyapaar-market-indicators` | All indicator modules + `screener` + `render`; tests extracted to `tests/`. |
| `honba-me-signals` | `vyapaar-market-signals` | Signal catalogue/detectors/engine (ADR 0031). |
| `honba-me-feed` | `vyapaar-market-feed` + `vyapaar-market-store` | Store owns lakehouse/recorder/bhavcopy/export/import; feed was scaffolded and later re-homed into `core`/`store`. |
| `honba-me-loaders` | `vyapaar-market-loaders` | Scaffolded; loading actually lives in `store` (`ParquetBarSource`) and `normalizer`. |
| `honba-me-zerodha` | `vyapaar-market-zerodha` | Empty scaffold (real Kite parser not ported). |
| `honba-me-upstox` | `vyapaar-market-upstox` | Empty scaffold. |
| `honba-me-fyers` | `vyapaar-market-fyers` | Empty scaffold. |
| `honba-me-angelone` | `vyapaar-market-angelone` | Empty scaffold. |
| — (new) | `vyapaar-market-pricing` | Black-Scholes `PricingModel` (plan 0001 Phase 2). |
| — (new) | `vyapaar-market-dhan` | `DhanFeed` skeleton implementing `MarketDataFeed`. |

## Migration Steps (as executed)

1. Workspace `members` + `workspace.dependencies` brought in
   `tokio`, `tokio-stream`, `tokio-tungstenite`, `flate2`, `arrow`, `parquet`,
   `rust_decimal`, `chrono`, `async-trait`, `futures`, `proptest`, plus
   `vyapaar-core`.
2. Bottom-up crate order: pure compute first (`finance`, `normalizer`,
   `indicators`, `signals`), then I/O (`core`, `store`), then scaffolds
   (`feed`, `loaders`, brokers).
3. `honba_core` → `vyapaar_core` and `honba_me_*` → `vyapaar_market_*`
   imports renamed everywhere.
4. Zero `#[cfg(test)]` blocks remain in `crates/*/src/` — tests moved to
   `tests/` parallel to `src/`.

## Verification Commands

```bash
# Run from the repository root
cargo fmt --check
cargo clippy --workspace --all-targets --profile prod -- -D warnings
cargo test --workspace --profile prod
```

## Migration Milestone Status

| Milestone | Status |
|-----------|--------|
| Pure compute crates (`finance`, `normalizer`, `indicators`, `signals`) | Complete |
| Feed/store (`core`, `store`) | Complete |
| Pricing (`vyapaar-market-pricing`) | Complete |
| Broker wire-mapping verification (plan 0001 Phase 3) | Complete — no wire-translation code exists in the 5 broker crates; table-driven `BrokerEnumMap<T>` documented as forward contract |
| Broker feed crates (`zerodha`/`upstox`/`fyers`/`angelone`) | **Stubs** (see `../gaps/0003-*`) |
| `feed`/`loaders` | **Stubs** (see `../gaps/0004-*`) |
| Zero inline `#[cfg(test)]` in `src/` | Complete |
| `cargo fmt --check` / clippy `-D warnings` | Clean |
| `cargo test --workspace --profile prod` | Passing |
