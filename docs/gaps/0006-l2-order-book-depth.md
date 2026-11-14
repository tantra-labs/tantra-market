# Gap 0006 — L2 Order-Book Depth Source Not Available

Severity: Medium · Status: Open · Tracked by: `tantra-agent/docs/plans/0007` Track 2 Phase 4

## What's Missing

The chat `market_depth` tool (L2 5-level order book with liquidity-imbalance
percentage, plan 0007 Track 2 Phase 4) has no backing data source: the broker
WebSocket feed crates (`tantra-market-dhan`, `tantra-market-zerodha`, and the
pending `upstox`/`fyers`/`angelone` — see gap 0003) currently carry tick/quote
payloads, not full L2 depth ladders, and no depth lakehouse exists.

## Requirements

1. A depth source: either an L2 depth tick path in a broker feed crate, or a
   depth snapshot lakehouse in `tantra-market-store` with a `DepthReader` port.
2. Wire the runtime `FeedMarketReader::market_depth` implementation to it.
3. Until then the runtime adapter answers `PortError::unavailable` citing this
   gap — never fabricated depth.

## Why It Matters

Liquidity imbalance and order-book depth are inputs to the institutional /
smart-money detection suite (agent plan 0010) and to market-quality chat
queries.
