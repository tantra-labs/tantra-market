# 0002 — Open-interest time-series history & buildup classification

**Status**: open

## What's stubbed or hardcoded

[0001](0001-options-chain-reader.md) already notes, in passing, that "open interest has no
source wired yet anywhere in this repo" when scoping the chain reader's per-contract OI field.
This gap is broader than that single-snapshot need: there is no OI **time series** anywhere in
`vyapaar-market` — no store, no reader, no DTO — for either options or futures contracts. A
repo-wide grep for `open_interest`/`OpenInterest` outside the options-chain gap's own
discussion finds no persisted history, only (once 0001 closes) a single current-moment value
per contract.

Classic institutional-positioning detection (the "smart money footprint" in F&O) requires
comparing OI *change* against price *change* over a window, classified into one of four
quadrants:

| Price | OI | Classification |
|---|---|---|
| Up | Up | Long buildup |
| Down | Up | Short buildup |
| Up | Down | Short covering |
| Down | Down | Long unwinding |

None of this is possible without persisted OI snapshots over time, which don't exist today for
either the options chain or index/stock futures.

## Why

`vyapaar-agent/docs/plans/0008-institutional-smart-money-detection-suite.md` scopes an
`oi_buildup` chat tool and uses OI-buildup direction as one leg of its composite smart-money
signal; both need this repo to be the actual data source, per this workspace's gap-tracking
rule (`.agents/rules/gap-tracking.md` §5): `vyapaar-market` owns instrument reference data and
derivatives pricing, so OI history belongs at this layer alongside the chain enumeration work
in [0001](0001-options-chain-reader.md), not duplicated in a consumer repo.

This gap is intentionally sequenced after [0001](0001-options-chain-reader.md): a chain
reader's design decision (which crate owns enumeration, whether a first-cut even sources OI)
determines where the periodic snapshot job for this gap would hook in. Futures OI is a
separate, additional source from the options chain (index/stock futures aren't chain
contracts), so this gap also covers wiring an OI reader for the futures instrument class, not
only options.

## How to close it

1. Close or substantially progress [0001](0001-options-chain-reader.md) first — it decides
   where/how a current OI value is sourced per contract, which this gap's periodic-snapshot job
   depends on.
2. Decide the futures-OI source (broker feed vs. exchange end-of-day bhavcopy) separately from
   the options-chain OI source, since futures contracts aren't part of an options chain.
3. Add a periodic snapshot store (e.g. `crates/vyapaar-market-store/src/open_interest.rs`)
   persisting `(instrument, as_of, open_interest: Quantity)` rows — exact `Decimal`-backed
   `Quantity`, never a float, per this repo's Zero Floats Policy.
4. Add a reader that, given an instrument and a lookback window, returns the OI delta and price
   delta over that window and classifies it into the four-quadrant buildup/unwinding/covering
   table above — this classification logic itself can live here or be left to the consumer
   (`vyapaar-strategies`); decide the boundary when implementing, rather than assuming it here.
5. Expose the reader to Rust consumers so `vyapaar-agent-chat-tools`'s planned `oi_buildup` tool
   (`vyapaar-agent/docs/plans/0008-institutional-smart-money-detection-suite.md` Phase 3) has a
   real read instead of a `PortError::unavailable` stand-in.

No code should be written against this gap document until the snapshot-store design (crate
ownership, snapshot cadence, futures-vs-options OI source) is decided; it is documentation
only, recording the decision point and target shape.
