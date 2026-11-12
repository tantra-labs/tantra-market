# `vyapaar-market-normalizer` — Instrument Master & Corporate Actions Reference

Documents `crates/vyapaar-market-normalizer/src/lib.rs`: in-memory instrument
master and corporate-actions engine. Pure, WASM-portable (file loading lives
in `vyapaar-market-loaders`/callers).

## `InstrumentMaster`

An in-memory multi-key lookup index over `InstrumentMasterEntry`
(`exchange`, `symbol`, `token`, `isin`, `instrument_type`, `lot_size`,
`tick_size`, `freeze_qty`).

| Method | Purpose |
|--------|---------|
| `by_token`, `by_symbol`, `by_isin` | O(1) multi-key resolution |
| `resolve_tick_size`, `resolve_lot_size` | Lookup from instrument rules |
| `load_zerodha_csv(bytes)` | Zerodha `instruments.csv` payload (accepts raw bytes) |
| `load_upstox_json(bytes)` | Upstox `complete.json.gz` payload |
| `load_nse_equity_csv(bytes)` | NSE `EQUITY_L.csv` payload |
| `import_master` / `export_master` | Serialize/deserialize the whole master |
| `entry` | Full entry by `InstrumentId` |

All loaders accept raw bytes so callers can feed downloaded or memory-mapped
master files without touching the OS filesystem (the crate stays
WASM-portable; file/network loading lives in `vyapaar-market-loaders`).

## `CorporateActions`

Per-ISIN registry of corporate-action events, sorted by effective date:

* `CorporateActionType::{Split, Bonus, Dividend, SymbolRename, Demerger}`
* `add_event`, `events_for_isin`
* `adjustment_factor(isin, as_of)` — cumulative multiplicative price factor
  (splits and bonuses adjust price; dividends/renames/demergers do not)
* `adjust_price(isin, historical_price, as_of)` — apply the factor

## Zero Floats Policy

Lot sizes are `u32`; tick sizes are exact `vyapaar_core::Price`;
`adjust_price` returns exact `Price`. No floats in the public API.

## Related

* `vyapaar-market-loaders` — file/network loading (currently an empty stub; the master loaders above are called with raw bytes).
* `vyapaar-core` — `Exchange`, `InstrumentId`, `Price`, `Quantity`.
