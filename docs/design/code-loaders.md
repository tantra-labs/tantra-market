# `vyapaar-market-loaders` — Historical Loaders Facade (Stub)

Documents `crates/vyapaar-market-loaders/`: an empty scaffolded crate.
`src/lib.rs` is a single doc-comment line (`//! vyapaar-market-loaders implementation.`).

## What it was meant to be

The old `honba-me-loaders` crate was to own historical EOD/intraday data
loaders. That responsibility was re-homed:

* **Parquet/history loading** (`ParquetBarSource`, `DatasetReader`,
  `CatalogInstrumentMaster`, lakehouse readers) lives in
  `vyapaar-market-store`.
* **Instrument-master loaders** in `vyapaar-market-normalizer` accept raw
  bytes directly (`load_zerodha_csv`, `load_upstox_json`, `load_nse_equity_csv`),
  so no filesystem-loading crate is needed on WASM targets.

## Status

Empty stub. Nothing in the workspace imports `vyapaar_market_loaders`.
Tracked by root gap `docs/gaps/0002-market-feed-loaders-stubs.md` and the
per-repo gap `../gaps/0004-feed-loaders-stubs.md` — removal is the likely
resolution.

## Related

* `code-store.md` — the actual history readers.
* `code-normalizer.md` — the actual master-loading entry points.
