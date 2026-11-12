# `vyapaar-market-core` — Feed Ports & Shared Transport Reference

Documents `crates/vyapaar-market-core/src/` (`lib.rs` + `ws_client.rs`): the
canonical market-data feed interface, symbol mapping, tick coalescing,
reconnection state, quote enrichment, and the shared WebSocket transport that
broker adapters build on.

## Module Map

| Module | Purpose |
|--------|---------|
| `lib.rs` | `MarketDataFeed` trait, `Mode`, `Subscription`, `SymbolMapper`, `CoalescingTickBuffer`, `ReconnectionManager`, `MarketDataEnricher`, `FeedHealth`, `MdError`, `ConnectionStatus` |
| `ws_client.rs` | `WsClient`, `WsConfig`, `WsMessage`/`WsOutMessage`, `WsConnectionState`, `WsError` — tokio-tungstenite + rustls transport with heartbeat, dead-connection detection, exponential-backoff reconnection and jitter |

## `MarketDataFeed` Trait

```rust
#[async_trait]
pub trait MarketDataFeed: Send + Sync {
    fn name(&self) -> &str;
    async fn subscribe(&self, subs: Vec<Subscription>)
        -> Result<BoxStream<'static, Tick>, MdError>;
    async fn unsubscribe(&self, instruments: &[InstrumentId]) -> Result<(), MdError>;
    async fn fetch_snapshot(&self, instruments: &[InstrumentId]) -> Result<Vec<Tick>, MdError>;
    fn feed_health(&self) -> FeedHealth;
}
```

Subscriptions carry an instrument plus a depth `Mode` (`Ltp`, `Quote`, `Full`).
`fetch_snapshot` is the REST reconciliation path used after a reconnect to
bridge missed ticks.

## Symbol Mapping

`SymbolMapper` maintains two dictionaries (broker key ↔ `InstrumentId`) plus
heuristic parsing of common broker spellings:

* `NSE:INFY` / `NSE:INFY-EQ` → `Exchange::Nse`, `INFY`
* `NSE_EQ|INE009A01021` (Upstox) → NSE with token
* `INFY$NSE` → `Exchange::Nse`, `INFY`

## Backpressure Coalescing

`CoalescingTickBuffer` is a bounded `tokio::sync::mpsc` queue with a
per-instrument last-value cache. When the channel saturates, LTP/Quote ticks
are coalesced into the cache instead of dropped; `drain_coalesced` flushes
them once the consumer catches up. Timestamps are enforced monotonic per
instrument (stale/out-of-order ticks are rejected).

## Reconnection Management

`ReconnectionManager` tracks `ConnectionStatus` (`Connected`,
`Disconnected`, `Reconnecting`, `Reconciling`) and retains the active
subscription set so a feed can auto-resubscribe after reconnect and reconcile
missed ticks via `fetch_snapshot`.

## Market-Data Enricher

`MarketDataEnricher` attaches dynamic scrip `PriceBand`s to quote ticks and
maintains a cumulative (price×volume, volume) accumulator to derive an
exact-decimal intraday VWAP.

## WebSocket Transport (`ws_client.rs`)

`WsClient` wraps `tokio-tungstenite` with `rustls-tls-native-roots`:

* heartbeat ping/pong on `WsConfig::heartbeat_interval_secs` (default 30 s),
* dead-connection detection via `last_message_elapsed` (default timeout 90 s),
* exponential backoff with full jitter up to `max_reconnect_delay_secs`,
* automatic resubscribe/reconnect state on the `WsConnectionState` enum.

Defaults: `max_reconnect_delay_secs = 60`, `initial_reconnect_delay_secs = 1`,
`max_message_size = 10 MiB`.

## Zero Floats Policy

All prices/quantities crossing these ports are canonical `vyapaar_core`
`Price`/`Quantity` (`rust_decimal`) — no floats.

## Related

* `code-brokers.md` — broker adapters implement `MarketDataFeed` on top of `WsClient`.
* `code-store.md` — `TickRecorder` subscribes to a broadcast stream and flushes to the lakehouse.
* `vyapaar-core` — canonical `Tick`, `Bar`, `InstrumentId`.
