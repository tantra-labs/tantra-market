# व्यापार・行情 tantra-market

> *Market Data: Multi-broker binary WebSockets, tick coalescing, 100+ technical indicators, and Parquet data loaders.*

High-throughput market data ingestion, normalization, and indicator calculation engine.

## Subcrates

- `tantra-market` — Unified market data facade
- `tantra-market-core` — Core feed traits, order book structures, and bar aggregates
- `tantra-market-feed` — Multi-provider connection multiplexer
- `tantra-market-normalizer` — Normalization of heterogeneous broker ticks into canonical `Tick`
- `tantra-market-indicators` — 100+ streaming technical indicators
- `tantra-market-signals` — Event-driven signal generators
- `tantra-market-store` — In-memory ring buffer & tick cache
- `tantra-market-loaders` — Parquet & CSV historical tick/bar loaders
- `tantra-market-finance` — Financial data series and macro feeds
- `tantra-market-zerodha` — Zerodha Kite ticker WebSocket client
- `tantra-market-upstox` — Upstox market data WebSocket client
- `tantra-market-angelone` — Angel One SmartAPI WebSocket client
- `tantra-market-fyers` — Fyers WebSocket market data client

## Build & Test

```bash
make prod
make check
make test
```

License: AGPL-3.0.
