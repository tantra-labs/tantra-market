# `vyapaar-market-store` — Lakehouse, Dataset & Readers Reference

Documents `crates/vyapaar-market-store/src/`: the Parquet lakehouse, the
immutable columnar dataset used by backtests, and the readers/exporters that
serve the async port traits.

## Module Map

| Module | Purpose |
|--------|---------|
| `lakehouse.rs` | **Lakehouse v2** — Hive-partitioned Parquet, `Decimal128(18,4)` prices, IST-partitioned, append-safe `SYMBOL.part-NNNN.parquet` |
| `dataset.rs` | `ColumnarSlice`, `ColumnarSliceBuilder`, `Dataset`, `DatasetFeed`/`StreamedDatasetFeed`, `Message`, `DataFeed` trait |
| `reader.rs` | `DatasetReader` — implements `InstrumentMaster`/`BarReader`/`QuoteReader`/`DepthReader` over a `Dataset` |
| `instruments.rs` | `CatalogInstrumentMaster` — dated `YYYY-MM-DD.parquet` snapshots with point-in-time `resolve_as_of` |
| `parquet_ticks.rs` | `ParquetTickStore` — legacy `Float64` tick store + `write_tick_file` (ADR 0030) |
| `live_quotes.rs` | `LiveQuoteStore` — writable latest-quote store over a `DatasetReader` fallback |
| `bhavcopy.rs` | `BhavcopyConnector` — `MarketDataConnector` replaying an NSE bhavcopy CSV as 1-day bars |
| `recorder.rs` | `TickRecorder` — async daemon flushing a tick stream to the lakehouse |
| `ticks.rs` | `InMemoryTickReader` — quote/trade ticks held in a `Vec` |
| `export/` | `ReportWriter` trait + `CsvReportWriter`, `JsonReportWriter`, `MarkdownReportWriter` (CSV/JSON/Markdown) over `vyapaar_agent_analytics::PerformanceReport` |
| `import/` | `ParquetBarSource` — parquet bar reader (`ts`, OHLCV float64) into `ColumnarSlice`/`Bar`s |
| `canonical.rs` | Bridge from legacy `f64` wire types to canonical `vyapaar_core` `Tick`/`Bar`; `migrate_tick_store`/`migrate_bars` |
| `wire.rs` | Legacy `f64` wire types: `Bar`, `QuoteTick`, `TradeTick`, `BarSpecification`, `BarType`, `UnixNanos`, `PriceType`, `BarAggregation`, `AggressorSide` |
| `ports.rs` | Async port traits: `BarReader`, `TickReader`, `QuoteReader`, `DepthReader`, `InstrumentMaster`, `MarketDataConnector` + `PortError` |

## Lakehouse v2 (`lakehouse.rs`)

```text
{root}/
├── ticks/exchange=NSE/date=2026-10-20/INFY.parquet
│                                      INFY.part-0001.parquet   (later flushes)
└── bars/timeframe=1m/exchange=NSE/year=2026/month=10/INFY.parquet
```

* Every price/quantity column is `Decimal128(18, 4)` — zero floats.
* Timestamps are `Timestamp(Nanosecond, "UTC")`; `date`/`year`/`month`
  partitions are the **IST** (Asia/Kolkata) calendar values of the event.
* Writes never modify an existing file: first write creates `SYMBOL.parquet`,
  later ones add `SYMBOL.part-NNNN.parquet`; files are written to a `*.tmp`
  sibling and renamed, so readers never see a half-written file.
* Depth ladders and price bands are JSON text columns.
* `read_ticks_range`/`read_bars_range` are range reads ascending by exchange
  time; `Lakehouse` implements `vyapaar_core::TickReader`/`BarReader`.
* Compression: `LakehouseCompression::{Snappy, Zstd (default), None}`.

## Dataset (`dataset.rs`)

Shared, immutable, columnar bars for one instrument — the read-only input a
backtest or parameter sweep runs against. `ColumnarSlice` holds one
`Arc<SliceInner>` (ts/open/high/low/close/volume columns); a clone shares them.
`Dataset` = ordered `Vec<ColumnarSlice>` by `InstrumentId`, cheap to clone
(`Arc`). `DatasetFeed`/`StreamedDatasetFeed` replay a dataset into the sync
event kernel in a fixed order, which is what makes sweep journals comparable
across thread counts.

## Readers

* `DatasetReader` — over an in-memory `Dataset`; implements `InstrumentMaster`
  (deterministic `InstrumentId` order), `BarReader` (exact range binary
  search), `QuoteReader` (derived bid=ask=last close, zero size),
  `DepthReader` (always `Unsupported` — bars carry no order book).
* `CatalogInstrumentMaster` — reads `{root}/instruments/YYYY-MM-DD.parquet`
  snapshots once at open; answers `get_instrument`, `list_instruments`,
  `snapshot(as_of)`, and point-in-time `resolve_as_of`. Snapshot columns
  include `symbol`, `exchange`, `kind`, `currency`, `lot_size`, `tick_size`,
  `freeze_qty`, `expiry`, `strike`, `option_type`, `underlying`; the numeric
  columns are read as `f64` and accept any numeric Arrow type.
* `ParquetTickStore` — legacy `Float64` tick store under `{root}/ticks/`,
  answers `TickReader` over `[from, to)`; `write_tick_file` lays out
  quotes-first-then-trades with `kind`/`ts_event`/`ts_init` columns.
* `LiveQuoteStore` — a `QuoteReader` whose latest-quote answer can be
  refreshed from a live feed (`put_quote`, `put_bar`); `as_of` queries keep
  replay semantics and fall back to the dataset.
* `InMemoryTickReader` — quote/trade ticks in a `Vec`, sorted by `ts_event`;
  an empty store answers `PortError::Unsupported`.

## Live Ingestion

* `BhavcopyConnector` — parses an NSE bhavcopy CSV (series `EQ`/`BE`/`BZ`)
  into 1-day bars stamped at 09:15 IST and replays them as `MarketEvent::Bar`
  through the `MarketDataConnector` port; no gaps, no reconnects.
* `TickRecorder` — background tokio task draining a broadcast/mpsc stream of
  canonical `Tick`s into rolling buffers and flushing to the `Lakehouse` when
  `max_ticks` (100k) or `flush_interval` (60 s) is reached, plus once on
  shutdown; parquet encoding runs on the blocking pool; failed flushes keep
  and retry their batch; buffer overflow drops the *oldest* ticks and counts
  them in `RecorderStats::dropped`.

## Exporters (`export/`)

`ReportWriter` serializes a `vyapaar_agent_analytics::PerformanceReport`:

* `CsvReportWriter` — two-column `metric,value` CSV.
* `JsonReportWriter` — compact or pretty JSON.
* `MarkdownReportWriter` — human-readable Markdown document.

Note: `export/mod.rs` declares only `csv`, `error`, `json`, `markdown`,
`writer` as modules. The files `excel.rs`, `parquet.rs` and `tearsheet.rs`
exist in the directory but are **not** declared as modules and are not
compiled — they are unwired placeholders from the original scaffold.

## Legacy Bridge (`canonical.rs`)

`decimal_from_f64` converts a finite `f64` to exact `Decimal` via the shortest
round-trip string (`1500.1` → `1500.1`, not `1500.0999999999999`);
`bar_to_canonical`, `quote_to_canonical`, `trade_to_canonical` lift legacy wire
types to canonical `Tick`/`Bar`; `migrate_tick_store`/`migrate_bars` rewrite
legacy stores into the lakehouse.

## Zero Floats Policy

The lakehouse is exact `Decimal128`; the legacy `wire.rs` types are the
documented `f64` exception for pre-lakehouse file formats, bridged to
canonical types by `canonical.rs` so downstream crates never see floats.

## Related

* `code-core.md` — `TickRecorder` attaches to a `vyapaar-market-core` broadcast stream.
* `vyapaar-agent-analytics` — `PerformanceReport` (cross-repo dependency).
* `vyapaar-core` — `Tick`, `Bar`, `TickReader`/`BarReader` ports.
