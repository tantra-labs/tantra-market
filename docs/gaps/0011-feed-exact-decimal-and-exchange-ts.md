# 0011 — Dhan/Kite decode prices via `from_f64_retain`; snapshots forge `exchange_ts = now`

**Status**: open
**Filed**: 2026-11-23 — workspace design review (root plan 0021) · **Kind**: design-flaw · **Severity**: high
**Plan**: [0006](../plans/0006-design-review-2026-11.md)

## What's stubbed or hardcoded

- `crates/tantra-market-dhan/src/lib.rs:258-259` `to_price(value: f64)` uses `Decimal::from_f64_retain(value).unwrap_or(Decimal::ZERO)` on an f64 that is a widened f32 (`crates/tantra-market-dhan/src/protocol.rs:20-21`). `f32 2450.1` becomes `2450.100097656...`: the exact binary expansion is kept and never rounded to the instrument tick (0.05 / 0.01).
- `crates/tantra-market-zerodha/src/lib.rs:361` applies the same `from_f64_retain(..).unwrap_or(Decimal::ZERO)` to REST `last_price`.
- `unwrap_or(Decimal::ZERO)` turns NaN/inf into a price of 0 instead of an error.
- Snapshot ticks stamp `exchange_ts: now`: `crates/tantra-market-dhan/src/lib.rs:426-430` and `crates/tantra-market-zerodha/src/lib.rs:146,363`, with `Utc::now()` at `crates/tantra-market-dhan/src/lib.rs:399` and `crates/tantra-market-zerodha/src/lib.rs:341`. Snapshot ticks therefore pass the `CoalescingTickBuffer` monotonicity guard, and the recorder partitions them (it keys on `exchange_ts`, `crates/tantra-market-store/src/lakehouse.rs:1017`) under the wrong IST date/time.

## Why

Non-tick-aligned prices flow into risk price bands/collars and recorded history: `2450.100097656` vs `2450.10` breaks equality, joins and golden vectors against the Python lakehouse. A zero price from NaN can pass a band check as a "reduce". Violates the exact-decimal intent of `.agents/rules/interface-first-and-modularity.md` §2.3 and the zero-clock rule for domain timestamps.

## How to close it

1. Add `round_to_tick(Decimal, tick_size)` (or `Decimal::from_f32_retain` then `round_dp(2/4)` per segment, currency 4 dp) in a shared `tantra-market-core` `wire` helper used by both brokers.
2. Return `MdError::Decode` on non-finite input; remove `unwrap_or(Decimal::ZERO)`.
3. Use the broker `last_trade_time` / response timestamp for snapshot `exchange_ts` (or make it `Option`).
4. Inject the clock (`Arc<dyn Clock>`) instead of calling `Utc::now()`.

**Verification**: golden-vector tests in the dhan and zerodha crates' `tests/` with f32 wire bytes; proptest that `round_to_tick` output is a tick multiple; NaN/inf negative tests. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, targeted `cargo test -p tantra-market-dhan --profile prod`, `make prod`.

## Related

- [0009](0009-f64-indicator-internals.md) covers indicators only, not feed decode.
- [0012](0012-coalescer-tick-loss.md) (monotonicity guard keyed on receive time).
- Lakehouse partition date depends on a correct `exchange_ts`; see [0014](0014-lakehouse-writer-ref-cas.md).
