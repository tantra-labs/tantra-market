# 0004 — `vyapaar-market-feed` and `vyapaar-market-loaders` are empty stubs

**Status**: open

Cross-reference: root gap `docs/gaps/0002-market-feed-loaders-stubs.md`
(provider repo `vyapaar-market`).

## What's stubbed or hardcoded

- `vyapaar-market-feed` — `src/lib.rs` is one doc-comment line. The actual
  market-data feed abstractions (`MarketDataFeed` trait,
  `CoalescingTickBuffer`, `ReconnectionManager`, `WsClient`, `Subscription`,
  `SymbolMapper`) all live in `vyapaar-market-core`.
- `vyapaar-market-loaders` — `src/lib.rs` is one doc-comment line.
  Lakehouse/history loading (`ParquetBarSource`, `DatasetReader`,
  `CatalogInstrumentMaster`, lakehouse readers) lives in
  `vyapaar-market-store`; instrument-master loaders live in
  `vyapaar-market-normalizer` and accept raw bytes.

## Why

Both crates were scaffolded from the old `honba-me-feed`/`honba-me-store` and
`honba-me-loaders` split (see `docs/ddd-conflict.md` Conflict #4), but the
split was later resolved by putting the feed traits in `vyapaar-market-core`
and the lakehouse in `vyapaar-market-store`. The two scaffolded crates were
never deleted, so the workspace carries two empty crates whose names imply
they own functionality that actually lives elsewhere.

## How to close it

1. Remove `vyapaar-market-feed` and `vyapaar-market-loaders` from
   `vyapaar-market/Cargo.toml` `members` and `workspace.dependencies`.
2. Verify no workspace imports `vyapaar_market_feed` /
   `vyapaar_market_loaders` in code (verified: none do).
3. Close root gap 0002.
4. Verify: `cd vyapaar-market && cargo check --workspace --profile prod` and
   `make test`.

## Verification

`cargo check --workspace --profile prod` passes without the crates;
`vyapaar-market-core` remains the single home of the feed trait;
`vyapaar-market-store` remains the single home of the lakehouse.
