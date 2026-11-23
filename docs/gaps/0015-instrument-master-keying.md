# 0015 — Normalizer `InstrumentMaster` indexes by bare symbol and empty-string token

**Status**: open
**Filed**: 2026-11-23 — workspace design review (root plan 0021) · **Kind**: design-flaw · **Severity**: high
**Plan**: [0006](../plans/0006-design-review-2026-11.md)

## What's stubbed or hardcoded

- `crates/tantra-market-normalizer/src/lib.rs:124-126` `symbol_index.insert(entry.symbol.clone(), id)` keys on the symbol only, so NSE `RELIANCE` and BSE `RELIANCE` (and NFO/NSE with the same underlying) overwrite each other. `by_symbol` (`crates/tantra-market-normalizer/src/lib.rs:94`) returns whichever loaded last.
- `crates/tantra-market-normalizer/src/lib.rs:121-125` `token.unwrap_or_default()`: entries without a token all insert at key `""`, and `with_token("")` / `with_isin("")` stamp empty strings into `InstrumentId`, so an instrument compares unequal to the same instrument built elsewhere with `token: None`.
- Three unrelated `InstrumentMaster` definitions: `tantra-core/src/ports.rs:154` (trait), `crates/tantra-market-store/src/ports.rs:313` (trait), `crates/tantra-market-normalizer/src/lib.rs:56` (struct); plus `CatalogInstrumentMaster` (`crates/tantra-market-store/src/instruments.rs:120`). `load_upstox_json` / `load_zerodha_csv` (`crates/tantra-market-normalizer/src/lib.rs:141,192`) duplicate Python `tantra-data` instrument ingestion and `tantra-adapters` symbol maps.
- The normalizer is a dependency of `tantra-runtime-api-rest` and `tantra-agent`, so the defect reaches user-facing lookups.

## Why

Symbol resolution for a cross-listed stock can return the wrong exchange's instrument, giving wrong lot, tick and freeze quantity in risk checks.

## How to close it

1. Key `symbol_index` by `(Exchange, symbol)`.
2. Return `Result` for a missing token; stop stamping empty strings.
3. Collapse onto one `InstrumentMaster` port in `tantra-core`, with the store catalog as the only implementation and the normalizer reduced to parsers.

**Verification**: proptest in `crates/tantra-market-normalizer/tests/` for insertion-order independence; cross-listed NSE/BSE fixture; `token: None` equality test. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, targeted `cargo test -p tantra-market-normalizer --profile prod`, `make prod`.

## Related

- Root gap 0019 (calendar, instrument master and corporate-action single sources).
- Root plan 0019 P4.3 moves types into store wire/ports; it does not cover these index defects.
