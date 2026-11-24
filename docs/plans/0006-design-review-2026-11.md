# 0006 — tantra-market design review (2026-11): resilient feeds, exact-decimal ingestion & object-store-ready store

**Status**: Proposed (2026-11-23)
**Scope**: `tantra-market` · **Umbrella**: root `docs/plans/0021-workspace-design-review-2026-11.md`

## 0. Executive Summary & Objective

The review found that the live feed path is not resilient: `WsClient` does not replay subscriptions, enforce its dead-connection timeout or refresh tokens, and logs the tokenised URL. Broker decode produces non-tick-aligned prices and forged snapshot timestamps, the coalescer loses ticks, and the stores and lakehouse writers assume a single local process with whole-dataset RAM loads. This plan fixes those defects, makes the store object-store-ready with safe concurrent commits, and fixes instrument keying. Non-goals: the unbounded-channel and type-relocation work already owned by root `docs/plans/0019-codebase-audit-refactoring-unification-and-wiring.md` (P1.5, P2.6, P4.1-P4.3).

## 1. Context & Gaps Covered

| Gap | Title | Kind | Severity |
|---|---|---|---|
| [0010](../gaps/0010-ws-client-resilience-and-token-logging.md) | WsClient resubscribe, dead-connection timeout, auth refresh, token logging | design-flaw | critical |
| [0011](../gaps/0011-feed-exact-decimal-and-exchange-ts.md) | Exact-decimal tick-aligned decode and real snapshot `exchange_ts` | design-flaw | high |
| [0012](../gaps/0012-coalescer-tick-loss.md) | `drain_coalesced` tick loss; coalescer not wired | design-flaw | high |
| [0013](../gaps/0013-store-memory-bounds-and-object-store.md) | Store memory bounds and object-store port | deployment | high |
| [0014](../gaps/0014-lakehouse-writer-ref-cas.md) | Lakehouse writer ref CAS and part naming | design-flaw | high |
| [0015](../gaps/0015-instrument-master-keying.md) | Instrument master keying by `(Exchange, symbol)` | design-flaw | high |
| [0016](../gaps/0016-rest-client-timeouts-and-errors.md) | REST client timeouts and error mapping | design-flaw | medium |

Plan-only items (no standalone gap doc):

- R1 — Feed port: failover, replay and sharding. `MarketDataFeed` (`crates/tantra-market-core/src/lib.rs:94-111`) returns an unfiltered shared broadcast per `subscribe`, has no sequence/gap model or `FeedEvent`, no failover arbiter, no `ReplayFeed`, and `KiteFeed` has a single `WsClient` (`crates/tantra-market-zerodha/src/lib.rs:50-60`) against Kite's per-key socket and 3000-instrument limits.
- R2 — Pricing numeric core. `crates/tantra-market-pricing/src/lib.rs:225-239` uses an A&S CDF (error < 7.5e-8) and `~366-379` an absolute 1e-9 IV tolerance with a fixed bracket; only Black-Scholes exists. Five duplicate erf/norm_cdf copies exist elsewhere in the workspace (agent, ui-core, core) plus Python `tantra-ffi/python/tantra/options.py:117-240`. See root gap 0015.
- R3 — Indicator layer. The `Indicator` trait (`crates/tantra-market-indicators/src/indicator.rs:10-30`) is not object-safe; constructors `assert!` on bad periods (`sma.rs`); orphan 1-line `mod.rs` stubs exist in six folders; Python and TS copies duplicate the Rust set (exact `Decimal` indicator internals landed 2026-11-24, closing gap 0009). See root gap 0016.
- R4 — `tantra-market-finance` scope: roughly 6k LOC of research studies in `crates/tantra-market-finance/src/studies/` sit in L2 and are consumed by `experimental_analysis.rs` in runtime; `tantra-ledger` (L1) depends on the whole crate (upward edge). See root plan 0019 P4.1/P4.2.
- R5 — Feed lifecycle and observability: `connect()` spawns decode tasks with dropped handles and no `CancellationToken`; `FeedHealth` lacks last-tick age and staleness; no metrics; `TickRecorder` is embedded in the REST process so N replicas would each open a broker socket and write the same lakehouse; Dhan URL constants are hard-coded (`crates/tantra-market-dhan/src/lib.rs:44-46`). See root plan 0022.

## 2. Component Specifications & Phases

### Phase A — Quick, low-risk fixes
1. Gap 0010 steps 3-5 and log redaction: deadline in `run_ws_loop`, backoff fix, real health counters, URL redaction.
2. Gap 0011: shared `round_to_tick` wire helper, non-finite decode error, broker timestamps for snapshots, injected clock.
3. Gap 0012: fix `drain_coalesced`, drop counters, exchange-time monotonicity with eviction.
4. Gap 0016: shared timeout-configured transport, status-to-error mapping, redacted credentials type.
5. Gap 0015: key by `(Exchange, symbol)`, no empty-string tokens.

**Gate**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, `cargo test -p tantra-market-core --profile prod`, `cargo test -p tantra-market-dhan --profile prod`, `cargo test -p tantra-market-zerodha --profile prod`, `cargo test -p tantra-market-normalizer --profile prod`, `make prod`.

### Phase B — Structural refactors
1. Gap 0010 steps 1-2: `WsHandshake` port and `on_connected` replay plus `fetch_snapshot` gap-fill.
2. Gap 0012 step 4 with R1: per-subscriber `CoalescingTickBuffer` publish, `FeedEvent` stream, `FailoverFeed`, `ReplayFeed`, socket sharding.
3. Gap 0014: `RefStore::compare_and_swap`, unique temp names, canonical-JSON golden vectors, shared schema files owned by `tantra-data`, cross-language round-trip test.
4. R2: single numeric core, relative-tolerance Newton/bisection IV, richer `PricingError`, `Black76Model`, golden vectors.
5. R3: `DynIndicator` / `IndicatorSpec`, `Result` constructors, remove orphan stubs, shared golden fixtures under `tests/golden/indicators/`.
6. R4: split `tantra-market-finance` into a thin L2 calendar/rules/costs crate and a studies crate placed in L3; add `tests/dependency_rules.rs`.

**Gate**: Phase A gate plus `cargo test -p tantra-market-store --profile prod`, `cargo test -p tantra-market-pricing --profile prod`, `cargo test -p tantra-market-indicators --profile prod`; cross-language lakehouse test run from `tantra-data` with `pytest -q`.

### Phase C — Deployment readiness
1. Gap 0013: `LakehouseFs` port with `LocalFs` and `object_store` impls, manifest-pruned lazy readers, byte-bounded LRU, `spawn_blocking` decode.
2. R5: `CancellationToken` through `connect`, health fields (`last_tick_age`, `reconnects`, `dropped`), metrics trait, optional `tantra-market-feedd` binary as a singleton per broker credential, endpoint overrides, design note on egress/leader constraints in `docs/design/`.

**Gate**: Phase B gate plus memory-bound and partition-pruning tests; `make prod`.

## 3. Relationship to Other Plans

- Not duplicated: root plan 0019 P1.5 (unbounded channels), P2.6 (projection generic), P4.1/P4.2/P4.3 (cost tables, `tantra-agent-analytics` edge, store types).
- Feeds root plan 0022 (singleton feed daemon, static egress, object-store lakehouse, readiness) and root plan 0023 (CI for this repo).
- Root gaps: 0011 (token leakage), 0015 (pricing), 0016 (indicator parity), 0018 (lakehouse contract), 0019 (instrument master). Provider-side: `tantra-data/docs/gaps/0027-bridge-export-drops-datasets-from-main.md`, `tantra-data/docs/gaps/0028-ref-cas-and-catalog-locking.md`.
- Existing: [0003](../gaps/0003-broker-websocket-feed-stubs.md), plan [0005](0005-concept-indicator-library.md); gap 0009 closed 2026-11-24.

## 4. Exit Criteria

1. A half-open socket, dropped connection and expired token each recover with subscriptions replayed and a snapshot gap-fill, covered by paused-time tests; no token appears in logs.
2. Broker-decoded prices are tick-aligned `Decimal`; NaN/inf yield `MdError::Decode`; snapshot ticks carry broker time.
3. For N ticks in, the last tick per instrument is delivered, and every drop is counted.
4. Two concurrent lakehouse writers (Rust and Python) never lose a commit; cross-language golden tests pass.
5. Readers open a month-scale fixture within a fixed memory bound and read via `LocalFs` or object store.
6. Cross-listed symbols resolve to the correct exchange; no empty-string tokens.
7. Snapshot REST calls time out and map 401/403/429/5xx distinctly.
8. R1-R5 are either implemented or re-filed as gaps; `make prod` is green.
