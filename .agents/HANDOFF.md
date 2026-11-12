# HANDOFF — `vyapaar-market`

## Phase / Slice Status
Plan `docs/plans/0001-technicals-breadth-and-derivatives-pricing.md` (Technical Indicator Breadth, Market Data Scope, and Derivatives Pricing) is **complete and cleaned up**:
- Phase 1 (indicator breadth: 18 indicators) — implemented & committed (`ddb5c3f`).
- Phase 2 (`vyapaar-market-pricing` Black-Scholes slice with `Decimal` Greeks) — implemented & committed (`1983512`).
- Phase 3 (broker wire-mapping verification) — verified: no wire-translation code exists in the 5 market broker crates (4 placeholders + 1 stub feed); nothing to refactor; table-driven `BrokerEnumMap<T>` documented as forward contract (`602a710`).
- Phase 4 (global market-data scope) — **decided**: decline global/68-country expansion by default; `vyapaar-market` remains India-only (5 brokers). Revisit only via a separate sign-off decision.

Plan document removed from active `docs/plans/`; entry moved to "Implemented Plans" in `docs/plans/README.md`.

## Decisions & Discoveries
- Zero-floats policy for pricing: Black-Scholes closed form + implied-vol root finder compute internally in `f64` (isolated in `vyapaar-market-pricing`); every result rounded to 8-decimal `Decimal` at the public API boundary; no `f64` crosses out of the crate or into `vyapaar-ledger`/`vyapaar-risk`.
- Build/test/lint all green on `prod` profile: `cargo test --workspace --profile prod`, `cargo clippy --workspace --all-targets --profile prod -- -D warnings`, `cargo fmt --check`.
- `docs/migration-guide.md` decimated: historical migration map moved to `docs/design/code-migration-map.md`; verification commands folded into the same map; original `migration-guide.md` removed. `docs/roadmap/` removed (dismantled into `docs/{design,gaps,plans}`).
- **Docs re-write (2026-11-12)**: `docs/design/` now has per-module `code-*.md` docs (core, finance, normalizer, indicators, signals, pricing, store, feed, loaders, brokers) reflecting the actual 14-crate workspace. `docs/gaps/` gained `0003` (broker WS feed stubs) and `0004` (feed/loaders stubs), cross-referencing root gaps 0001/0002.

## Immediate Next Step
None — no in-flight work in `vyapaar-market`. Next market-data work (when any broker crate gains real tick/instrument translation) must adopt the table-driven `BrokerEnumMap<T>` pattern per the forward contract in plan §2.4.

## Operating Invariants
- Canonical build profile is `prod`: `make prod` / `cargo test --workspace --profile prod`; no bare `cargo build` or `--release`.
- No `#[test]` in `src/`; all tests live in `tests/`.
- Zero floats in financial paths crossing crate boundaries.
