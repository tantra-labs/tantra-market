# `vyapaar-market` — DDD Architecture & Market Data Bounded Context

## 1. Bounded Context & Sub-Crate Partitioning
`vyapaar-market` (Tier 2A, ported and restructured from `honba-me`) provides market microstructure rules, incremental technical indicators, explainable alpha signals, live multi-broker WebSocket feeds, and Arrow/Parquet lakehouse storage.

| Sub-Crate | Layer Type | Responsibility |
| :--- | :--- | :--- |
| `vyapaar-market-core` | Domain / Port | `MarketDataFeed` trait, `SymbolMapper`, `CoalescingTickBuffer`, `ReconnectionManager`, WebSocket transport primitives. |
| `vyapaar-market-finance` | Pure Domain | Generic market contracts (`MarketProfile`, `MarketCalendar`, `CostSchedule`, `InstrumentRules`, `SettlementRules`, `ExpiryRules`) + `india` market pack (`NseCalendar`, `CostModel`, `SttRates`, `FnoContract`, `Nifty50`). |
| `vyapaar-market-normalizer` | Pure Domain | Scrip normalization, ISIN/token resolution, corporate action adjustment rules. |
| `vyapaar-market-indicators` | Pure Compute | 100+ streaming & vectorized technical indicators (`Sma`, `Ema`, `Rsi`, `Atr`, `Supertrend`, `Vwap`, `Cpr`, `Adx`, `Ichimoku`, etc.). |
| `vyapaar-market-signals` | Pure Compute | Multi-horizon (`Intraday`, `Day`, `Swing`) alpha signal confluence and screener AST evaluation. |
| `vyapaar-market-feed` | Async Stream | Live quote/tick fan-out, price-band enrichment, bhavcopy & instrument master loaders. |
| `vyapaar-market-store` | Storage Infra | Hive-partitioned Apache Arrow / Parquet lakehouse (`Decimal128`), `TickRecorder`, CSV/Excel/Parquet/Markdown exporters. |
| `vyapaar-market-loaders` | Ingestion Infra| Historical EOD/intraday data loaders and fixture importers. |
| `vyapaar-market-{zerodha,upstox,fyers,angelone}` | Broker ACL | Binary WebSocket protocol decoders translating broker frames into `vyapaar_core::Tick`. |
