# 0013 — Tick/OI/instrument stores load whole datasets into RAM, block at open, and have no object-store abstraction

**Status**: open
**Filed**: 2026-11-23 — workspace design review (root plan 0021) · **Kind**: deployment · **Severity**: high
**Plan**: [0006](../plans/0006-design-review-2026-11.md)

## What's stubbed or hardcoded

- `crates/tantra-market-store/src/parquet_ticks.rs:134-150` `open()` reads every `*.parquet` under `ticks/` into `Vec<QuoteTick>/Vec<TradeTick>/Vec<DepthTick>`. `read_quotes` (`crates/tantra-market-store/src/parquet_ticks.rs:191-201`) is a linear scan with `.cloned()`. There is no date/instrument pruning although the Hive layout (`exchange=/date=/symbol.parquet`, `crates/tantra-market-store/src/lakehouse.rs:1012-1035`) allows it.
- `crates/tantra-market-store/src/open_interest.rs:218-250` eagerly loads all OI files into a `BTreeMap`; `crates/tantra-market-store/src/instruments.rs:130` (`open`) eagerly `read_dir`s the whole catalog (`:194`).
- 20+ direct `std::fs::File::open/read_dir` call sites across `crates/tantra-market-store/src`; no `ObjectStore`/`opendal` trait. Only `recorder.rs:137` uses `spawn_blocking`. `open()` is called from async bootstrap (runtime `market_feed.rs`).
- `Cargo.toml` has `arrow`/`parquet` but no `object_store`; paths are `&Path` everywhere.

## Why

One month of NSE F&O full-depth ticks is tens of GB: `ParquetTickStore::open` will OOM or stall startup. A Kubernetes replica with ephemeral disk, or an S3/GCS-backed lakehouse (the `tantra-data` object layout), cannot be read at all. Breaks the bounded-memory intent of `.agents/rules/scalability-and-concurrency.md` §2-3.

## How to close it

1. Introduce a `LakehouseFs` port (`list(prefix)`, `get_range`, `put_atomic`) with a `LocalFs` impl and an `object_store`-backed impl.
2. Make readers lazy: resolve partitions from manifest entries (`min_ts`/`max_ts` already recorded in `ManifestEntry`), stream row groups with parquet predicate pushdown.
3. Add an LRU cache bounded by bytes.
4. Run blocking decode in `spawn_blocking`.

**Verification**: temp-dir partition-pruning test; memory-bound test with large synthetic fixtures; object-store parity test against `LocalFs`. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, targeted `cargo test -p tantra-market-store --profile prod`, `make prod`.

## Related

- Root plan 0019 P2.6 (projection generic) covers this only partially; root plan 0022 (data at scale).
- [0014](0014-lakehouse-writer-ref-cas.md) (object stores need conditional-put commit).
- `tantra-market-store` still depends on `tantra-agent-analytics` (root plan 0019 P4.2), which blocks edge/WASM reader builds.
