# Gaps Index — `tantra-market`

Every time something in this repo is stubbed, mocked, or hardcoded because the
real dependency (a data source, a pipeline, a design decision) isn't wired up
yet, it gets a numbered entry here instead of being left silent. Per root
`.agents/rules/gap-tracking.md` §5, this index also carries gaps whose symptom
shows up in a *different* repo but whose root cause lives here — and it is the
**implementation tracker** for provider-side gaps that are additionally filed
at the workspace root for cross-repo visibility.

## Open gaps

| # | Title | Area |
|---|-------|------|
| [0003](0003-broker-websocket-feed-stubs.md) | Real broker WS feeds — **feature tracker, 2 of 5 shipped**: `tantra-market-dhan` (DhanHQ v2, commit `fb82571`) and `tantra-market-zerodha` (Kite Connect 3, commit `ad79205`) are real `MarketDataFeed` implementations; `upstox`, `fyers`, `angelone` remain to be re-added as real crates when their wire protocols are implemented | `crates/tantra-market-{dhan,zerodha}` + re-added on implementation |

## Closed gaps

| # | Title | Closed In |
|---|-------|-----------|
| 0001 | Options-chain enumeration/reader — contract-reference + Greeks building blocks exist, but nothing enumerates/serves a chain (blocked chat `options_chain` tool) | Closed 2026-11-13 (T3.1): `InstrumentMaster::list_chain` on `CatalogInstrumentMaster`/`DatasetReader`, `ChainReader` port + `InstrumentChainReader`, exact-Decimal `ChainContract`; wired into `tantra-runtime-api-rest::FeedMarketReader::options_chain`. Commits `b1b085e`/`39ff389`. |
| 0002 | Open-interest time-series history & buildup classification — no persisted OI history for options or futures (blocked chat `oi_buildup` tool) | Closed 2026-11-13 (T3.2): `OpenInterestStore` port, `InMemoryOpenInterestStore` + `ParquetOpenInterestStore` (`{root}/open_interest/YYYY-MM-DD.parquet`, exact Decimal128 `Quantity`), `classify_buildup` four-quadrant + `BuildupAnalyzer`. Commit `b1b085e`. |
| 0004 | Real `feed`/`loaders` implementations — **feature tracker**: empty stub crates removed 2026-11-13 (root plan 0003); feed traits in `core`, loaders in `store` (root gap 0002, task T3.4) | Closed 2026-11-13 (T3.4): feed traits live in `tantra-market-core`, lakehouse/loaders live in `tantra-market-store`, and real `MarketDataFeed` implementations now exist on top of core (`dhan` `fb82571`, `zerodha` `ad79205`). |

## Related

- Root index: `../../../docs/gaps/README.md` (gap 0001 `market-broker-feed-stubs`,
  now 2-of-5 shipped; gap 0002 `market-feed-loaders-stubs` closed).
- [`docs/design/architecture.md`](../design/architecture.md) — crate status table.
