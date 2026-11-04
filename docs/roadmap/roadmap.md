# `vyapaar-market` — TDD & Engineering Roadmap

## Milestone 1: Pure Domain & Compute Crates (P0)
- [ ] Port `vyapaar-market-finance` (from `honba-me-market`), `vyapaar-market-normalizer`, `vyapaar-market-indicators`, and `vyapaar-market-signals`.
- [ ] Verify zero `tokio` dependencies in `indicators`, `signals`, `normalizer`, and `finance` for WASM compatibility.

## Milestone 2: Core Feed, Lakehouse Store & Loaders (P0)
- [ ] Port `vyapaar-market-core` and `vyapaar-market-loaders`.
- [ ] Split `honba-me-feed` cleanly into `vyapaar-market-feed` (live streaming & canonical feed ports) and `vyapaar-market-store` (Parquet lakehouse, tick recorder, multi-format exporters).

## Milestone 3: Broker Anti-Corruption Layer (ACL) Adapters (P0)
- [ ] Port `vyapaar-market-zerodha`, `vyapaar-market-upstox`, `vyapaar-market-fyers`, and `vyapaar-market-angelone`.
- [ ] Verify all unit, property, and golden tests pass in `<crate>/tests/`.
