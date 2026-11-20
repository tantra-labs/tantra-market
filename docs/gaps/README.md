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
|---|---|---|
| [0003](0003-broker-websocket-feed-stubs.md) | Real broker WS feeds — **feature tracker, 2 of 5 shipped**: `tantra-market-dhan` (DhanHQ v2, commit `fb82571`) and `tantra-market-zerodha` (Kite Connect 3, commit `ad79205`) are real `MarketDataFeed` implementations; `upstox`, `fyers`, `angelone` remain to be re-added as real crates when their wire protocols are implemented | `crates/tantra-market-{dhan,zerodha}` + re-added on implementation |
| [0007](0007-adr-pct-and-chandelier-exit.md) | ADR%(20) and Chandelier Exit indicators missing (trade-signal-generator presets; root plan 0008) | `crates/tantra-market-indicators` |
| [0009](0009-f64-indicator-internals.md) | f64 indicator internals — `atr.rs` blocking (Predictive Ranges); others f64-legacy, audit in T22.1 (root plan 0016 §4.3) | `crates/tantra-market-indicators` |

## Closed gaps

| # | Title | Closed In |
|---|---|---|
| 0008 | Lakehouse v3 CAS/manifest resolver, post-close `part-NNNN` compaction, `CatalogInstrumentMaster` `Float64 → Decimal128(18, 4)` migration, and unified `derivatives/`/`reference/`/`disclosures/`/`fundamentals/` layout alignment (root ADR `0004`, root plan `0011`) | Closed 2026-11-18 (T17.3): `ManifestResolver` (`refs/{heads,tags}/*`, `commits/*.json`, `trees/*.json`, `objects/sha256`, `views/` fallback on `main`, `reference/aliases` point-in-time rename continuity, `arrow_utf8_array` `Utf8`/`LargeUtf8` decoding) + post-close CAS compaction (`Lakehouse::compact_v3` preserving pinned tags) + `CatalogInstrumentMaster` `Decimal128(18, 4)` schema migration (`open_legacy_float64` opt-in helper) + `ParquetOpenInterestStore` (`derivatives/open_interest`) and all 9 projection readers (`block_deals`, `corporate_actions`, `delivery`, `economic_calendar`, `filings`, `fundamentals`, `funds_ipos`, `institutional_flows`, `news`) wired through `ManifestResolver`; Python `ManifestStore` atomic `trees/<hash>.json` export + rollback (`tantra-data` `test_cas_manifest` 13 passed); `tantra-market-store` `lakehouse_v3` (3) + `instruments` (5) + `chain_oi_tests` (10) + doctests (2) + `fmt --check` + `clippy -D warnings` + `make prod` green. |
| 0005 | No Elephant-zone / OI-wall / gamma-profile / Order-Block indicators (Elephant Edge EE+ level feeds) | Closed 2026-11-17 (T12.2, `44528ba`): `ElephantZones` (percentile + opening-range-extension session bands, `ElephantZonesValue {s1,s2,r1,r2}`), `OiLevels` (top-2 CE/PE walls + ΔOI score + Local Range PCR, `OiLevelsValue`), `GammaProfile` (GEX/net/flip reusing `tantra_core::options::GammaExposure`, `GammaProfileValue`), `OrderBlock` (displacement + last opposing candle zone) in `crates/tantra-market-indicators` — Decimal-only, zero-I/O; tests `elephant_zones_unit`/`oi_levels_unit`/`gamma_profile_unit`/`order_block_unit` + proptests. |
| 0001 | Options-chain enumeration/reader — contract-reference + Greeks building blocks exist, but nothing enumerates/serves a chain (blocked chat `options_chain` tool) | Closed 2026-11-13 (T3.1): `InstrumentMaster::list_chain` on `CatalogInstrumentMaster`/`DatasetReader`, `ChainReader` port + `InstrumentChainReader`, exact-Decimal `ChainContract`; wired into `tantra-runtime-api-rest::FeedMarketReader::options_chain`. Commits `b1b085e`/`39ff389`. |
| 0002 | Open-interest time-series history & buildup classification — no persisted OI history for options or futures (blocked chat `oi_buildup` tool) | Closed 2026-11-13 (T3.2): `OpenInterestStore` port, `InMemoryOpenInterestStore` + `ParquetOpenInterestStore` (`{root}/open_interest/YYYY-MM-DD.parquet`, exact Decimal128 `Quantity`), `classify_buildup` four-quadrant + `BuildupAnalyzer`. Commit `b1b085e`. |
| 0004 | Real `feed`/`loaders` implementations — **feature tracker**: empty stub crates removed 2026-11-13 (root plan 0003); feed traits in `core`, loaders in `store` (root gap 0002, task T3.4) | Closed 2026-11-13 (T3.4): feed traits live in `tantra-market-core`, lakehouse/loaders live in `tantra-market-store`, and real `MarketDataFeed` implementations now exist on top of core (`dhan` `fb82571`, `zerodha` `ad79205`). |
| 0006 | No L2 order-book depth source for the chat `market_depth` tool (plan 0007 Track 2 Phase 4) | Closed 2026-11-14: exact-Decimal `DepthSnapshot` port (prices/sizes `Price`/`Quantity`, no floats) + two data sources implementing it — `LiveQuoteStore::put_depth` + `DepthReader` (live depth store, mirrors `put_trade`) and `ParquetTickStore` depth ticks (`write_depth_file`, kind `"depth"` + `depth_json` column; real broker full-mode `Tick::Depth` frames — zerodha 10 levels, dhan 5 — are the lakehouse input); runtime `FeedMarketReader::with_depth_reader` + real `market_depth` ladder + `liquidity_imbalance_pct` (closes `tantra-runtime/docs/gaps/0007` item 10) |

## Related

- Root index: `../../../docs/gaps/README.md` (gap 0001 `market-broker-feed-stubs`,
  now 2-of-5 shipped; gap 0002 `market-feed-loaders-stubs` closed).
- [`docs/design/architecture.md`](../design/architecture.md) — crate status table.
