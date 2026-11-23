# 0012 — `drain_coalesced` drops ticks it could not send; coalescer and enricher are dead code

**Status**: open
**Filed**: 2026-11-23 — workspace design review (root plan 0021) · **Kind**: design-flaw · **Severity**: high
**Plan**: [0006](../plans/0006-design-review-2026-11.md)

## What's stubbed or hardcoded

- `crates/tantra-market-core/src/lib.rs:256-275` `drain_coalesced`: on `try_send` failure the loop keeps the tick "in cache" per its comment, but `cache.clear()` then runs unconditionally, so unsent latest-value ticks are discarded and `drained` under-counts.
- `crates/tantra-market-core/src/lib.rs:229-235` the out-of-order guard compares `receive_ts` (local clock) instead of exchange time and drops silently (`return Ok(())`, no counter). `last_timestamps` / `last_tick_per_instrument` `HashMap`s grow with every instrument ever seen and are never evicted.
- `CoalescingTickBuffer`, `MarketDataEnricher` (`crates/tantra-market-core/src/lib.rs:345`) and `ConnectionStatus` have no non-test consumer in the workspace (only `core/tests/enricher.rs`).
- `DhanFeed`/`KiteFeed` broadcast raw ticks via `broadcast::channel(16_384)` and swallow `Lagged` (`crates/tantra-market-dhan/src/lib.rs:328`, `crates/tantra-market-zerodha/src/lib.rs:273`: `filter_map(|r| r.ok())`) and ignore send errors (`crates/tantra-market-dhan/src/lib.rs:130`, `crates/tantra-market-zerodha/src/lib.rs:107`). `dropped_count` is hard-coded 0.

## Why

On a burst (expiry day) a slow consumer silently loses data, and the documented "latest value" coalescing guarantee (AGENTS.md, tick coalescing and backpressure) is not applied in any live path. Lagging subscribers lose ticks with no metric.

## How to close it

1. Fix `drain_coalesced` to retain entries that failed `try_send` and clear only the sent ones.
2. Count drops (guard drops, lag, full channel) into `FeedHealth.dropped_count`.
3. Key monotonicity on `exchange_ts` with per-instrument eviction (or a bounded LRU).
4. Publish from `DhanFeed`/`KiteFeed` through a per-subscriber `CoalescingTickBuffer` with explicit `BackpressurePolicy::CoalesceLatest` instead of a lossy broadcast.

**Verification**: proptest in `crates/tantra-market-core/tests/` that for N ticks in, the last tick per instrument is always delivered and drain counts are exact; lag/drop counter tests. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, targeted `cargo test -p tantra-market-core --profile prod`, `make prod`.

## Related

- Root plan 0019 P1.5 touches the same channels (boundedness), not these semantics.
- [0011](0011-feed-exact-decimal-and-exchange-ts.md) (snapshot `exchange_ts` correctness feeds the monotonicity guard).
- Plan item R1 (per-subscriber filtering and `FeedEvent` stream).
