# 0010 — WsClient never resubscribes, enforces no dead-connection timeout, cannot refresh auth, and logs the token

**Status**: open
**Filed**: 2026-11-23 — workspace design review (root plan 0021) · **Kind**: design-flaw · **Severity**: critical
**Plan**: [0006](../plans/0006-design-review-2026-11.md)

## What's stubbed or hardcoded

- `crates/tantra-market-core/src/ws_client.rs:2-5` module doc promises "dead-connection detection ... automatic subscription replay on reconnect"; neither exists.
- `run_ws_loop` (`crates/tantra-market-core/src/ws_client.rs:~314-420`) only sends Ping on an interval. `dead_connection_timeout_secs` is read only by the passive helper `is_dead()` (`crates/tantra-market-core/src/ws_client.rs:302`), which has no caller in the workspace. A half-open TCP socket (no FIN) stalls the feed forever.
- `crates/tantra-market-dhan/src/lib.rs:96-98` and `crates/tantra-market-zerodha/src/lib.rs:85` state the client "replays no subscriptions by itself". After the first reconnect the broker socket has zero subscriptions while `connected` stays true. `ReconnectionManager.get_resubscribe_list()` is used only by `unsubscribe` (`crates/tantra-market-dhan/src/lib.rs:336`, `crates/tantra-market-zerodha/src/lib.rs:280`). `fetch_snapshot` gap-fill is never invoked.
- Auth is baked into the URL once: `crates/tantra-market-dhan/src/lib.rs:~76` (`?version=2&token=...`), `crates/tantra-market-zerodha/src/lib.rs:~68`. `WsClient.ws_url: String` is immutable, so a daily-expiring Kite/Dhan token cannot be rotated; every reconnect after expiry loops on 401 forever.
- `crates/tantra-market-core/src/ws_client.rs:215` logs `info!("WebSocket connected: {}", ws_url)`; the Kite/Dhan feed URLs carry the access token in the query string, so the token is written to logs.
- Backoff: jitter is applied after the sleep and can yield 0 (`(1*2).min(60) as f64*(0.5+j)` cast to u64, `crates/tantra-market-core/src/ws_client.rs:269-271`), giving a tight reconnect loop. `Reconnecting { attempt: 1 }` is hard-coded (`crates/tantra-market-core/src/ws_client.rs:254,262`). Connect never gives up (`Failed` is unreachable). `feed_health()` always returns `last_heartbeat: None, reconnect_count: 0` in both broker crates.

## Why

After any network blip, or at 09:15 IST, the feed goes silent while `feed_health().connected == true`. Strategies and risk read stale prices with no signal. A token leaked into logs grants order-capable API access. Breaks `.agents/rules/security-and-secrets.md` §3.3 (secrets never logged).

## How to close it

1. Add a `WsHandshake` port (`async fn url(&self) -> Result<Url>` plus headers) invoked on every (re)connect so the broker adapter can refresh tokens; redact the URL (strip query) in all logs.
2. Add an `on_connected` hook that replays `ReconnectionManager` subscriptions, then calls `fetch_snapshot`.
3. Enforce `dead_connection_timeout_secs` inside `run_ws_loop` with a `select!` deadline.
4. Fix backoff: jitter before sleep, minimum 1 s, real attempt counter, `max_attempts` leading to `Failed`.
5. Populate `FeedHealth` (`last_heartbeat`, `reconnect_count`) from real counters.

**Verification**: tests in `tests/` using tokio paused time and a local tungstenite server (drop connection, half-open, expired token, log capture asserting no token); proptest on backoff bounds. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, targeted `cargo test -p tantra-market-core --profile prod`, `make prod`.

## Related

- [0003](0003-broker-websocket-feed-stubs.md) tracks broker feature coverage, not these defects.
- Root gap 0011 (credential/token leakage); root plan 0019 P1.5 covers unbounded channels only.
- Plan item R1 (feed port failover/sharding) builds on the handshake/health work here.
