# `vyapaar-market` — Historical Migration Map (honba-me → vyapaar-market)

> **Tier**: Tier 2A (Depends on `vyapaar-core`)
> **Source**: `honba-workspace/honba-me`
> **Target**: `vyapaar-market`
> **Status**: Migration complete.

## Sub-Crate Mapping Table

| Source Crate (`honba-me/crates/`) | Target Crate (`vyapaar-market/crates/`) | Notes & DDD Refactoring |
| :--- | :--- | :--- |
| `honba-me-core` | `vyapaar-market-core` | Port `lib.rs`, `ws_client.rs`, and `tests/`. |
| `honba-me-market` | `vyapaar-market-finance` | Rename package `honba-me-market` → `vyapaar-market-finance`. Extract `src/tests` into `tests/`. |
| `honba-me-normalizer` | `vyapaar-market-normalizer` | Port normalizer and instrument master mapping. |
| `honba-me-indicators` | `vyapaar-market-indicators` | Port all indicator modules (`sma`, `ema`, `rsi`, `supertrend`, `vwap`, `cpr`, etc.). Extract any inline tests to `tests/`. |
| `honba-me-signals` | `vyapaar-market-signals` | Port signal generators (`intraday`, `day`, `swing`, `screener`). |
| `honba-me-feed` | `vyapaar-market-feed` + `vyapaar-market-store` | `vyapaar-market-store` gets `lakehouse.rs`, `parquet_ticks.rs`, `recorder.rs`, `export/`, `import/`, `dataset.rs`. `vyapaar-market-feed` gets `live_quotes.rs`, `canonical.rs`, `ports.rs`, `reader.rs`, `bhavcopy.rs`, `instruments.rs` (and re-exports `vyapaar-market-store` for backward compatibility). |
| `honba-me-loaders` | `vyapaar-market-loaders` | Port historical data loaders. |
| `honba-me-zerodha` | `vyapaar-market-zerodha` | Port Kite binary WebSocket parser. |
| `honba-me-upstox` | `vyapaar-market-upstox` | Port Upstox v2 binary/Protobuf feed adapter. |
| `honba-me-fyers` | `vyapaar-market-fyers` | Port Fyers v3 binary data socket adapter. |
| `honba-me-angelone` | `vyapaar-market-angelone` | Port Angel One SmartAPI 2.0 WebSocket adapter. |

## Step-by-Step Migration Instructions

1. Update `vyapaar-market/Cargo.toml` `[workspace.dependencies]` to include all external crates from `honba-me/Cargo.toml` (`tokio`, `tokio-stream`, `tokio-tungstenite`, `flate2`, `arrow`, `parquet`, `rust_decimal`, `chrono`, `async-trait`, `futures`, `proptest`) plus internal workspace crates (`vyapaar-market-core`, `vyapaar-market-finance`, etc.).
2. Migrate in bottom-up crate order inside `vyapaar-market`:
   - Step 1: `vyapaar-market-core`, `vyapaar-market-finance`, `vyapaar-market-normalizer`
   - Step 2: `vyapaar-market-indicators`, `vyapaar-market-signals`
   - Step 3: `vyapaar-market-store`, `vyapaar-market-feed`, `vyapaar-market-loaders`
   - Step 4: `vyapaar-market-{zerodha,upstox,fyers,angelone}`
3. Replace all `honba_core` → `vyapaar_core` and `honba_me_*` → `vyapaar_market_*` imports.
4. Ensure zero `#[cfg(test)]` blocks remain in `crates/*/src/`.

## Verification Commands

```bash
# Run from the repository root
grep -rn "#\[cfg(test)\]" crates/*/src/ && exit 1 || true
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```
