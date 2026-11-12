# `tantra-market-feed` — Live Feed Facade (Stub)

Documents `crates/tantra-market-feed/`: an empty scaffolded crate. `src/lib.rs`
is a single doc-comment line (`//! tantra-market-feed implementation.`).

## What it was meant to be

The old `honba-me-feed` split promised a live streaming/multiplexing feed
crate. That responsibility was later re-homed:

* **Feed ports** (`MarketDataFeed`, `CoalescingTickBuffer`,
  `ReconnectionManager`, `WsClient`, `Subscription`, `SymbolMapper`) live in
  `tantra-market-core`.
* **Lakehouse / bhavcopy / recorder** live in `tantra-market-store`.

## Status

Empty stub. Nothing in the workspace imports `tantra_market_feed`.
Tracked by root gap `docs/gaps/0002-market-feed-loaders-stubs.md` and the
per-repo gap `../gaps/0004-feed-loaders-stubs.md` — removal is the likely
resolution (the crate's name implies ownership that actually lives in `core`
and `store`).

## Related

* `code-core.md` — the actual feed trait/transport.
* `code-store.md` — the actual live ingestion (`TickRecorder`, `BhavcopyConnector`).
