# 0016 — Snapshot REST clients: per-call `reqwest::Client`, no timeout, all non-2xx mapped to `Auth`

**Status**: open
**Filed**: 2026-11-23 — workspace design review (root plan 0021) · **Kind**: design-flaw · **Severity**: medium
**Plan**: [0006](../plans/0006-design-review-2026-11.md)

## What's stubbed or hardcoded

- `crates/tantra-market-dhan/src/lib.rs` and `crates/tantra-market-zerodha/src/lib.rs` build a new `reqwest::Client` per `fetch_snapshot` call with the default (unbounded) timeout.
- `api_key` / snapshot REST client construction lacks a pooled `HttpTransport` and zeroizing secret wrapper (token rotation and redacted `Debug` on `DhanFeed` / `KiteFeed` / `StaticTokenProvider` landed in closed gap 0010).
- Every non-2xx maps to `MdError::Auth`, including 429 and 5xx.
- No integration with the `tantra-exec` token bucket; REST quote limits are per broker.

## Why

A hung snapshot call during reconciliation blocks resubscription. A 429 is reported as an auth failure and could trigger a bogus re-login. Unpooled clients allocate connections per snapshot, and plain strings lack `Zeroize` (`.agents/rules/security-and-secrets.md` §3.3, §4.3).

## How to close it

1. Add a shared `HttpTransport` port in `tantra-market-core` (builder with 10 s timeout, pooled client).
2. Introduce a credentials type with redacted `Debug` and `Zeroize`.
3. Map 401/403 to `Auth`, 429 to a new `RateLimited { retry_after }`, 5xx to `Transport`.

**Verification**: wiremock tests in the broker crates' `tests/` (timeout, 401, 429, 503); `Debug` redaction test. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, targeted `cargo test -p tantra-market-dhan --profile prod`, `make prod`.

## Related

- Root gap 0011 (credential/token leakage across Rust and Python).
- Closed gap [0010](../gaps/README.md#closed-gaps) (WS handshake/resilience/URL redaction closed 2026-11-25).
