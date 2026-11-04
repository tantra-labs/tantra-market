# व्यापार・行情 vyapaar-market

> *Market Data: Multi-broker binary WebSockets, tick coalescing, 100+ technical indicators, and Parquet data loaders.*

High-throughput market data ingestion, normalization, and indicator calculation engine.

## Subcrates

- `vyapaar-market` — Unified market data facade
- `vyapaar-market-core` — Core feed traits, order book structures, and bar aggregates
- `vyapaar-market-feed` — Multi-provider connection multiplexer
- `vyapaar-market-normalizer` — Normalization of heterogeneous broker ticks into canonical `Tick`
- `vyapaar-market-indicators` — 100+ streaming technical indicators
- `vyapaar-market-signals` — Event-driven signal generators
- `vyapaar-market-store` — In-memory ring buffer & tick cache
- `vyapaar-market-loaders` — Parquet & CSV historical tick/bar loaders
- `vyapaar-market-finance` — Financial data series and macro feeds
- `vyapaar-market-zerodha` — Zerodha Kite ticker WebSocket client
- `vyapaar-market-upstox` — Upstox market data WebSocket client
- `vyapaar-market-angelone` — Angel One SmartAPI WebSocket client
- `vyapaar-market-fyers` — Fyers WebSocket market data client

## Build & Test

```bash
make prod
make check
make test
```

License: AGPL-3.0.
