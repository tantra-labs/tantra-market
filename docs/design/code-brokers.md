# `vyapaar-market` — Broker WebSocket ACL Crates Reference

Documents the five broker crates: `vyapaar-market-{zerodha,upstox,fyers,angelone,dhan}`.
Four are empty scaffolds; `dhan` has a skeletal feed adapter. This is the
Anti-Corruption Layer that is supposed to translate broker binary frames into
canonical `vyapaar_core::Tick`.

## Status

| Crate | Broker | Status |
|-------|--------|--------|
| `vyapaar-market-zerodha` | Zerodha Kite (binary WebSocket) | **Empty stub** (`src/lib.rs` = one doc-comment line) |
| `vyapaar-market-upstox` | Upstox v2 (binary/Protobuf feed) | **Empty stub** |
| `vyapaar-market-fyers` | Fyers v3 (binary data socket) | **Empty stub** |
| `vyapaar-market-angelone` | Angel One SmartAPI 2.0 (WebSocket) | **Empty stub** |
| `vyapaar-market-dhan` | Dhan (WebSocket feed) | **Skeleton** — `DhanFeed` struct implementing `MarketDataFeed`; `subscribe` returns an empty stream (no Dhan wire protocol) |

All five are registered in `vyapaar-market/Cargo.toml` `[workspace] members`
and `[workspace.dependencies]`, but nothing in any workspace imports them.

## The Dhan Skeleton

```rust
pub struct DhanFeed {
    pub client_id: String,
    pub access_token: String,
    pub ws_client: WsClient,
    pub mapper: Arc<RwLock<SymbolMapper>>,
    pub reconnection: Arc<ReconnectionManager>,
}
```

`DhanFeed::new(client_id, access_token)` builds the struct with
`DEFAULT_WS_URL = "wss://api.dhan.co/feed"`. Its `MarketDataFeed`
implementation:

* `name()` → `"dhan"`,
* `subscribe(_subs)` → returns an empty `BoxStream` from a fresh
  `CoalescingTickBuffer(5000)`,
* `unsubscribe`/`fetch_snapshot` → no-op / empty,
* `feed_health()` → `connected: false` with zero counters.

It exercises the `vyapaar-market-core` plumbing (`WsClient`, `SymbolMapper`,
`ReconnectionManager`) but decodes no Dhan frames.

## Why they exist as stubs

The workspace was scaffolded with all planned broker adapters up front so the
crate list would match the architecture doc; the real binary wire formats were
never populated. Tracked by root gap
`docs/gaps/0001-market-broker-feed-stubs.md` and the per-repo gap
`../gaps/0003-broker-websocket-feed-stubs.md`. Per gap-tracking rule, do not
re-add an empty stub once removed; add a crate only when its WS wire protocol
is actually implemented.

## The intended adapter shape

When implemented, each broker crate must build on `vyapaar-market-core`:

* `WsClient` for the binary transport (heartbeat + reconnect),
* `SymbolMapper` for broker token ↔ canonical `InstrumentId`,
* `CoalescingTickBuffer` for backpressure,
* `ReconnectionManager` for auto-resubscribe + REST snapshot reconciliation,
* and must translate frames **immediately** into canonical `vyapaar_core::Tick`.

## Related

* `code-core.md` — the feed trait/transport these adapters implement.
* `../gaps/0003-broker-websocket-feed-stubs.md` — open per-repo gap.
