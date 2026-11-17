# 0008 — Lakehouse v3 Manifest Reader, Compaction & Decimal Instrument Snapshot Alignment

**Status**: open

## What's stubbed or hardcoded

`crates/tantra-market-store/src/` currently implements Lakehouse v2 and several ad-hoc snapshot directories that diverge from root ADR [`docs/design/0004-data-catalog-lakehouse-layout.md`](../../../docs/design/0004-data-catalog-lakehouse-layout.md):

1. `lakehouse.rs` (`Lakehouse`) writes and reads only `{root}/ticks/exchange={EX}/date=YYYY-MM-DD/{SYMBOL}[.part-NNNN].parquet` and `{root}/bars/timeframe={tf}/exchange={EX}/year=YYYY/month=MM/{SYMBOL}.parquet` directly from filesystem globs — no `commit_sha` / `refs/` resolution, no `objects/sha256/` CAS lookup, and no nightly compaction of `part-NNNN.parquet` flushes.
2. `instruments.rs` (`CatalogInstrumentMaster`, `snapshot_schema`) reads `{root}/instruments/YYYY-MM-DD.parquet` with **`DataType::Float64`** for `lot_size`, `tick_size`, `freeze_qty`, and `strike`, breaking the Zero-Floats (`Decimal128(18, 4)`) invariant.
3. `open_interest.rs` (`ParquetOpenInterestStore`) reads `{root}/open_interest/YYYY-MM-DD.parquet` separately rather than under `derivatives/open_interest/exchange={EX}/year=YYYY/month=MM`, and projection readers (`filings`, `corporate_actions`, `block_deals`, etc.) read flat `{root}/catalog/<dataset>/` directories instead of v3 manifests.

## Why

- Rust live recorders (`TickRecorder`), backtests, and projection readers must share the exact same canonical v3 categorization, CAS commit pinning, and `Decimal128(18, 4)` schemas as `tantra-data` so Python-written data and Rust-written live captures seamlessly unify and compact cleanly.
- Keeping `Float64` in `CatalogInstrumentMaster` violates `tantra-labs` Zero-Floats discipline on contract tick sizes and option strikes.

## How to close it

1. Add a `ManifestResolver` (reading `refs/{heads,tags}/*`, `commits/<sha>.json`, and CAS `objects/sha256/<ab>/<hash>.parquet` or DuckDB manifest export) to `tantra-market-store`, keeping v2 Hive paths as a temporary read fallback behind a deprecation flag.
2. Add post-close compaction in `lakehouse.rs` that merges live `TickRecorder` `SYMBOL.part-NNNN.parquet` segments into `(instrument_uid, exchange_ts)`-sorted CAS blobs and commits cleanly.
3. Migrate `CatalogInstrumentMaster` (`snapshot_schema`) numeric columns (`lot_size`, `tick_size`, `freeze_qty`, `strike`) to `Decimal128(18, 4)` (accepting legacy `Float64` files only during explicit migration tests), and align `ParquetOpenInterestStore` + projection readers to `derivatives/` / `disclosures/` / `fundamentals/` / `reference/` v3 paths.
4. Verify cross-language Parquet read/write parity against `tantra-data` plan `0009` with `cargo test --profile prod` and `clippy -D warnings`.
