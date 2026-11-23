# 0016 — Snapshot REST clients: per-call `reqwest::Client`, no timeout, all non-2xx mapped to `Auth`

**Status**: open
**Filed**: 2026-11-23 — workspace design review (root plan 0021) · **Kind**: design-flaw · **Severity**: medium
**Plan**: [0006](../plans/0006-design-review-2026-11.md)

## What's stubbed or hardcoded

- `crates/tantra-market-dhan/src/lib.rs:378` and `crates/tantra-market-zerodha/src/lib.rs:319` build a new `reqwest::Client` per `fetch_snapshot` call with the default (unbounded) timeout.
- The access token is stored as a plain `String` field in `DhanFeed` / `KiteFeed` (no redacted `Debug`, no zeroize).
- Every non-2xx maps to `MdError::Auth` (`crates/tantra-market-dhan/src/lib.rs:~385-391`, `crates/tantra-market-zerodha/src/lib.rs:~326-331`), including 429 and 5xx.
- No integration with the `tantra-exec` token bucket; REST quote limits are per broker.

## Why

A hung snapshot call during reconciliation blocks resubscription. A 429 is reported as an auth failure and could trigger a bogus re-login. Plain-`String` tokens risk leaking through `Debug` output (`.agents/rules/security-and-secrets.md` §3.3, §4.3).

## How to close it

1. Add a shared `HttpTransport` port in `tantra-market-core` (builder with 10 s timeout, pooled client).
2. Introduce a credentials type with redacted `Debug` and `Zeroize`.
3. Map 401/403 to `Auth`, 429 to a new `RateLimited { retry_after }`, 5xx to `Transport`.

**Verification**: wiremock tests in the broker crates' `tests/` (timeout, 401, 429, 503); `Debug` redaction test. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, targeted `cargo test -p tantra-market-dhan --profile prod`, `make prod`.

## Related

- Root gap 0011 (credential/token leakage across Rust and Python).
- [0010](0010-ws-client-resilience-and-token-logging.md) (token in WS URL and logs).
