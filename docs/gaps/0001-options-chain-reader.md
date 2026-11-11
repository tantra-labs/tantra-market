# 0001 — Options-chain enumeration/reader

**Status**: open

## What's stubbed or hardcoded

`vyapaar-agent`'s plan 0006 Phase 4 adds an `options_chain` chat tool
(`vyapaar-agent-chat-tools`'s `MarketReader::options_chain` port method), and
`vyapaar-runtime`'s chat adapter answers it with an honest `PortError::unavailable`
stand-in because no chain reader exists in this repo to serve it.

Verified 2026-11-12 (point-in-time re-check, per the plan's §4 Phase 4 requirement):
the **building blocks** for an options chain all exist here, but nothing enumerates or
serves a chain as a set of contracts with quotes:

- **Pricing**: `crates/vyapaar-market-pricing/src/lib.rs` has `Greeks` (all five
  Greeks as exact `Decimal`, line ~156), `BlackScholesModel` (line ~193) with an
  `implied_volatility` root-finder (lines ~181/~345), and
  `PriceParams::from_fno_contract` (line ~92) that builds pricing input straight
  from a parsed contract's strike/expiry/call-put.
- **Contract symbol grammar**: `crates/vyapaar-market-finance/src/india/fno/mod.rs`
  has `OptionType` (CE/PE, line ~21) and `FnoContract::Option { underlying, expiry,
  strike, option_type }` (lines ~160–171) with a parse/format roundtrip for
  `NIFTY24DEC24000CE`-style symbols.
- **Queryable contract table**: `crates/vyapaar-market-store/src/instruments.rs`
  persists `expiry`/`strike`/`option_type`/`underlying`/`lot_size` columns in the
  instrument-master Arrow schema (lines ~56–59) — but `list_instruments()`
  (line ~193) only offers a flat/by-ID listing; there is **no filter-by-underlying
  chain query**.
- A repo-wide grep for `option_chain`/`OptionChain` finds **zero matches**: no
  reader, service, or DTO produces a chain for a downstream consumer.

So the runtime chat adapter cannot answer "show me the NIFTY option chain" — it
reports the gap explicitly instead of the model fabricating a chain.

## Why

Per this workspace's gap-tracking rule (`.agents/rules/gap-tracking.md` §5), a
stub's root cause is filed in the provider repo. The chat tool's root blocker is
here: `vyapaar-market` owns instrument reference data and derivatives pricing, so
the chain enumeration belongs at this layer, where `vyapaar-market-store` already
holds the contract-reference columns `vyapaar-market-pricing` can price.

## How to close it

1. Add a chain query to `vyapaar-market-store`'s instrument master: filter
   instruments by `underlying` + option `kind`, ordered by `expiry`, then
   `strike`, then `option_type` — the columns already exist (lines ~56–59).
2. Wrap it in a reader/service that joins each contract with its quote fields
   (last/bid/ask from the existing quote feed) and open interest — open interest
   has **no source wired yet** anywhere in this repo; decide whether OI is in
   scope for the first cut or a later addition rather than assuming a feed.
3. Optionally enrich per-contract rows with `Greeks`/implied vol from
   `vyapaar-market-pricing` via `PriceParams::from_fno_contract`.
4. Expose the reader to Rust consumers so `vyapaar-runtime`'s
   `FeedMarketReader::options_chain` (currently a stand-in) can replace its
   `PortError::unavailable` with a real read.

No code should be written against this gap document until the chain-reader
design (which crate owns the enumeration, and whether OI is in scope) is decided;
it is documentation only, recording the decision point and target shape.
