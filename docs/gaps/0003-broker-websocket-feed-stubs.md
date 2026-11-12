# 0003 — Broker WebSocket feed crates are empty stubs

**Status**: open

Cross-reference: root gap `docs/gaps/0001-market-broker-feed-stubs.md`
(provider repo `vyapaar-market`).

## What's stubbed or hardcoded

`vyapaar-market-zerodha`, `vyapaar-market-upstox`, `vyapaar-market-fyers` and
`vyapaar-market-angelone` each have a `src/lib.rs` that is a single doc-comment
line with no implementation. `vyapaar-market-dhan` has a `DhanFeed` struct
implementing `vyapaar_market_core::MarketDataFeed` (plus `WsClient`,
`SymbolMapper`, `ReconnectionManager` plumbing and `wss://api.dhan.co/feed`),
but its `subscribe` returns an empty stream — no Dhan WebSocket wire protocol
is decoded. Nothing in any workspace imports these five crates.

## Why

The workspace was scaffolded with all planned broker adapters up front so the
crate list would match the architecture doc. The real brokers (Zerodha Kite,
Upstox, Fyers, Angel One, Dhan) have binary WebSocket wire formats that are
non-trivial; the crates were never populated. `vyapaar-market-core` already
provides everything an adapter needs (`WsClient`, `CoalescingTickBuffer`,
`ReconnectionManager`, `SymbolMapper`), so implementing a real adapter is
wiring a decoder, not building infrastructure.

## How to close it

1. Remove `-zerodha`, `-upstox`, `-fyers`, `-angelone`, `-dhan` from
   `vyapaar-market/Cargo.toml` `members` and `workspace.dependencies` (one
   commit, atomic).
2. Do **not** delete the plan to add real broker feeds — re-add each crate
   only when its WS wire protocol is actually implemented (gap-tracking §4:
   deleting a stub is not closing the feature, but re-adding an empty stub is
   forbidden).
3. Close root gap 0001.
4. Verify: `cd vyapaar-market && cargo check --workspace --profile prod` and
   `make test`.

## Verification

`cargo check --workspace --profile prod` passes without the crates present; no
other workspace declares a path dependency on the removed crates.
