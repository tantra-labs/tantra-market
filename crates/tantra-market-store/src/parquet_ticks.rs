//! A Parquet-backed tick store: the persistent half of the quote/trade tick input an
//! `fill_model = l2` run matches against (ADR 0030, "no parquet tick store yet").
//!
//! [`ParquetTickStore`] reads a directory of tick files — `{root}/ticks/*.parquet`, read in
//! file-name order and merged — and answers the [`TickReader`] port as of a half-open
//! `[from, to)` range. The layout leaves room for a later date- or instrument-partitioned
//! directory; today every file under `ticks/` is read.
//!
//! [`write_tick_file`] is the producer: it lays out the quotes and trades it is given (quotes
//! first, each kind in input order) as one Parquet file. The reader sorts by `ts_event` (stable
//! for ties), so the writer's order only decides ties.
//!
//! # Tick columns
//!
//! Every row is one tick tagged by `kind` (`quote` or `trade`). `symbol`, `exchange` and
//! `kind` are UTF8; `ts_event` and `ts_init` are unix nanoseconds as `Int64` (`ts_init` is
//! nullable and defaults to `ts_event`); `bid_price`, `ask_price`, `bid_size`, `ask_size`,
//! `price` and `size` are `Float64`; `aggressor` (one of `buyer`, `seller`, `no_aggressor`)
//! and `trade_id` are UTF8. The columns a row's kind does not use are null. A quote row must
//! carry the four quote prices/sizes; a trade row its price, size, aggressor and trade id.
//!
//! The reader is `Send + Sync` and does all I/O once, at [`open`](ParquetTickStore::open), like
//! [`CatalogInstrumentMaster`](crate::CatalogInstrumentMaster); every answer is a pure
//! function of the loaded ticks. A store with no files, or files with no ticks, answers
//! [`PortError::Unsupported`] rather than inventing an empty series.

use std::fs::File;
use std::path::Path;
use std::str::FromStr;

use arrow::array::{Array, ArrayRef, Float64Array, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use rust_decimal::Decimal;
use tantra_core::{Exchange, InstrumentId, Price, Quantity};

use crate::ports::{DepthReader, DepthSnapshot, PortError, PortResult, TickReader, TickRequest};
use crate::wire::{AggressorSide, QuoteTick, TradeId, TradeTick, UnixNanos};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use thiserror::Error;

/// The directory under a store root that holds the tick Parquet files.
pub const TICK_DIR: &str = "ticks";

/// The column layout of a tick file, so producers and consumers agree.
pub fn tick_schema() -> Schema {
    Schema::new(vec![
        Field::new("kind", DataType::Utf8, false),
        Field::new("symbol", DataType::Utf8, false),
        Field::new("exchange", DataType::Utf8, false),
        Field::new("ts_event", DataType::Int64, false),
        Field::new("ts_init", DataType::Int64, true),
        Field::new("bid_price", DataType::Float64, true),
        Field::new("ask_price", DataType::Float64, true),
        Field::new("bid_size", DataType::Float64, true),
        Field::new("ask_size", DataType::Float64, true),
        Field::new("price", DataType::Float64, true),
        Field::new("size", DataType::Float64, true),
        Field::new("aggressor", DataType::Utf8, true),
        Field::new("trade_id", DataType::Utf8, true),
        Field::new("depth_json", DataType::Utf8, true),
    ])
}

/// Why a tick store could not be opened, written or read.
#[derive(Debug, Error)]
pub enum TickStoreError {
    /// The store directory or one of its files could not be read or written.
    #[error("cannot access tick store: {0}")]
    Io(#[from] std::io::Error),
    /// A `.parquet` file could not be decoded.
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// An Arrow array or batch could not be built.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// A required column is missing.
    #[error("missing column: {0}")]
    MissingColumn(&'static str),
    /// A column had an unexpected data type.
    #[error("column {name} has type {got:?}, which is not usable here")]
    WrongType {
        /// Name of the column.
        name: &'static str,
        /// Actual data type found.
        got: DataType,
    },
    /// A row is not a valid tick.
    #[error("{file}: row {row}: {message}")]
    BadRow {
        /// Name of the file the row came from.
        file: String,
        /// Zero-based row index within that file.
        row: usize,
        /// Why the row is not a tick.
        message: String,
    },
}

impl From<TickStoreError> for PortError {
    fn from(error: TickStoreError) -> Self {
        PortError::Internal(error.to_string())
    }
}

/// One persisted order-book depth snapshot (a `kind = "depth"` tick row).
#[derive(Clone, Debug, PartialEq)]
pub struct DepthTick {
    /// Instrument the book belongs to.
    pub instrument: InstrumentId,
    /// When the snapshot was taken.
    pub ts_event: UnixNanos,
    /// The order-book snapshot.
    pub snapshot: DepthSnapshot,
}

/// An in-memory view of a directory of tick Parquet files (ADR 0030).
#[derive(Clone, Debug, Default)]
pub struct ParquetTickStore {
    quotes: Vec<QuoteTick>,
    trades: Vec<TradeTick>,
    depth: Vec<DepthTick>,
}

impl ParquetTickStore {
    /// Loads every `*.parquet` file under `{root}/ticks/`.
    ///
    /// This blocks on I/O once, at start-up. Files are read in name order; ticks from every
    /// file are merged and sorted by `ts_event` (stable, so a tie keeps file and row order).
    /// A missing store directory is an error; a present but empty store answers
    /// [`PortError::Unsupported`] on every read.
    pub fn open(root: &Path) -> Result<Self, TickStoreError> {
        let dir = root.join(TICK_DIR);
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "parquet") {
                paths.push(path);
            }
        }
        paths.sort();
        let mut quotes = Vec::new();
        let mut trades = Vec::new();
        let mut depth = Vec::new();
        for path in &paths {
            read_file(path, &mut quotes, &mut trades, &mut depth)?;
        }
        quotes.sort_by_key(QuoteTick::ts_event);
        trades.sort_by_key(TradeTick::ts_event);
        depth.sort_by_key(|d| d.ts_event);
        Ok(Self {
            quotes,
            trades,
            depth,
        })
    }

    /// The quotes, trades and depth snapshots loaded, for tests and inspection.
    pub fn len(&self) -> (usize, usize, usize) {
        (self.quotes.len(), self.trades.len(), self.depth.len())
    }

    /// Borrowed quotes and trades, for the canonical bridge in [`crate::canonical`].
    pub(crate) fn raw(&self) -> (&[QuoteTick], &[TradeTick]) {
        (&self.quotes, &self.trades)
    }

    /// Returns `true` when the store holds no ticks at all.
    pub fn is_empty(&self) -> bool {
        self.quotes.is_empty() && self.trades.is_empty() && self.depth.is_empty()
    }

    fn check_held(&self) -> PortResult<()> {
        if self.is_empty() {
            return Err(PortError::Unsupported(
                "this store holds no tick data".into(),
            ));
        }
        Ok(())
    }
}

fn in_range(t: UnixNanos, request: &TickRequest) -> bool {
    request.from().is_none_or(|f| t >= f) && request.to().is_none_or(|e| t < e)
}

#[async_trait]
impl TickReader for ParquetTickStore {
    async fn read_quotes(&self, request: &TickRequest) -> PortResult<Vec<QuoteTick>> {
        self.check_held()?;
        Ok(self
            .quotes
            .iter()
            .filter(|q| {
                q.instrument_id() == request.instrument() && in_range(q.ts_event(), request)
            })
            .cloned()
            .collect())
    }

    async fn read_trades(&self, request: &TickRequest) -> PortResult<Vec<TradeTick>> {
        self.check_held()?;
        Ok(self
            .trades
            .iter()
            .filter(|t| {
                t.instrument_id() == request.instrument() && in_range(t.ts_event(), request)
            })
            .cloned()
            .collect())
    }
}

/// The latest persisted order-book snapshot for an instrument, truncated to `levels`.
///
/// The parquet lakehouse's depth ticks (written by [`write_depth_file`] from live broker
/// full-mode `Tick::Depth` frames — zerodha's ten entries, dhan's five) are the real L2
/// source this store serves (closes `tantra-market/docs/gaps/0006`).
#[async_trait]
impl DepthReader for ParquetTickStore {
    async fn read_depth(&self, id: &InstrumentId, levels: usize) -> PortResult<DepthSnapshot> {
        if self.depth.is_empty() {
            return Err(PortError::Unsupported(
                "this store holds no depth ticks".into(),
            ));
        }
        let latest = self.depth.iter().rev().find(|tick| tick.instrument == *id);
        match latest {
            Some(tick) => {
                let mut snapshot = tick.snapshot.clone();
                snapshot.bids.truncate(levels);
                snapshot.asks.truncate(levels);
                Ok(snapshot)
            }
            None => Err(PortError::Unsupported(format!(
                "{id} has no order-book depth in this store"
            ))),
        }
    }
}

/// Writes `quotes` then `trades`, in the order given, as one tick file at `path`.
///
/// Creates the parent directory if needed and overwrites an existing file. The reader sorts by
/// `ts_event`, so the order here only decides ties.
pub fn write_tick_file(
    path: &Path,
    quotes: &[QuoteTick],
    trades: &[TradeTick],
) -> Result<(), TickStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut kinds = Vec::with_capacity(quotes.len() + trades.len());
    let mut symbols = Vec::with_capacity(quotes.len() + trades.len());
    let mut exchanges = Vec::with_capacity(quotes.len() + trades.len());
    let mut ts_events = Vec::with_capacity(quotes.len() + trades.len());
    let mut ts_inits = Vec::with_capacity(quotes.len() + trades.len());
    let mut bid_prices = Vec::with_capacity(quotes.len() + trades.len());
    let mut ask_prices = Vec::with_capacity(quotes.len() + trades.len());
    let mut bid_sizes = Vec::with_capacity(quotes.len() + trades.len());
    let mut ask_sizes = Vec::with_capacity(quotes.len() + trades.len());
    let mut prices = Vec::with_capacity(quotes.len() + trades.len());
    let mut sizes = Vec::with_capacity(quotes.len() + trades.len());
    let mut aggressors = Vec::with_capacity(quotes.len() + trades.len());
    let mut trade_ids = Vec::with_capacity(quotes.len() + trades.len());

    for q in quotes {
        kinds.push("quote");
        symbols.push(q.instrument_id().symbol.clone());
        exchanges.push(exchange_str(q.instrument_id().exchange));
        ts_events.push(q.ts_event().as_u64() as i64);
        ts_inits.push(Some(q.ts_init().as_u64() as i64));
        bid_prices.push(Some(q.bid_price()));
        ask_prices.push(Some(q.ask_price()));
        bid_sizes.push(Some(q.bid_size()));
        ask_sizes.push(Some(q.ask_size()));
        prices.push(None);
        sizes.push(None);
        aggressors.push(None);
        trade_ids.push(None);
    }
    for t in trades {
        kinds.push("trade");
        symbols.push(t.instrument_id().symbol.clone());
        exchanges.push(exchange_str(t.instrument_id().exchange));
        ts_events.push(t.ts_event().as_u64() as i64);
        ts_inits.push(Some(t.ts_init().as_u64() as i64));
        bid_prices.push(None);
        ask_prices.push(None);
        bid_sizes.push(None);
        ask_sizes.push(None);
        prices.push(Some(t.price()));
        sizes.push(Some(t.size()));
        aggressors.push(Some(aggressor_name(t.aggressor_side())));
        trade_ids.push(Some(t.trade_id().as_str()));
    }

    let ints = |values: Vec<i64>| -> ArrayRef { std::sync::Arc::new(Int64Array::from(values)) };
    let opt_ints =
        |values: Vec<Option<i64>>| -> ArrayRef { std::sync::Arc::new(Int64Array::from(values)) };
    let floats =
        |values: Vec<Option<f64>>| -> ArrayRef { std::sync::Arc::new(Float64Array::from(values)) };
    let strings =
        |values: Vec<&str>| -> ArrayRef { std::sync::Arc::new(StringArray::from(values)) };
    let owned_strings = |values: Vec<String>| -> ArrayRef {
        std::sync::Arc::new(StringArray::from_iter_values(values))
    };
    let opt_strings =
        |values: Vec<Option<&str>>| -> ArrayRef { std::sync::Arc::new(StringArray::from(values)) };
    let opt_owned = |values: Vec<Option<String>>| -> ArrayRef {
        std::sync::Arc::new(StringArray::from_iter(values))
    };
    let null_depth_jsons = |n: usize| -> ArrayRef {
        std::sync::Arc::new(StringArray::from_iter(std::iter::repeat_n(None::<&str>, n)))
    };

    let n = kinds.len();
    let columns: Vec<ArrayRef> = vec![
        strings(kinds),
        owned_strings(symbols),
        strings(exchanges),
        ints(ts_events),
        opt_ints(ts_inits),
        floats(bid_prices),
        floats(ask_prices),
        floats(bid_sizes),
        floats(ask_sizes),
        floats(prices),
        floats(sizes),
        opt_owned(aggressors),
        opt_strings(trade_ids),
        null_depth_jsons(n),
    ];
    let schema = std::sync::Arc::new(tick_schema());
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    let mut writer = ArrowWriter::try_new(File::create(path)?, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

/// Serializes an order-book snapshot to the `depth_json` tick column.
///
/// Prices and sizes are string-encoded exact decimals, so no `f64` crosses the
/// parquet boundary (the zero-floats policy).
fn depth_json(snapshot: &DepthSnapshot) -> String {
    let level = |levels: &[(Price, Quantity)]| -> Vec<serde_json::Value> {
        levels
            .iter()
            .map(|(price, qty)| {
                serde_json::json!([price.value().to_string(), qty.value().to_string()])
            })
            .collect()
    };
    serde_json::json!({
        "bids": level(&snapshot.bids),
        "asks": level(&snapshot.asks),
    })
    .to_string()
}

/// Parses a `depth_json` tick column value back into an exact-decimal snapshot.
fn parse_depth_json(raw: &str, file: &str, row: usize) -> Result<DepthSnapshot, TickStoreError> {
    let bad = |message: String| TickStoreError::BadRow {
        file: file.to_string(),
        row,
        message,
    };
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| bad(format!("invalid depth_json: {e}")))?;
    let level =
        |side: &serde_json::Value, name: &str| -> Result<Vec<(Price, Quantity)>, TickStoreError> {
            let arr = side
                .as_array()
                .ok_or_else(|| bad(format!("depth_json {name} is not an array")))?;
            let mut out = Vec::with_capacity(arr.len());
            for entry in arr {
                let pair = entry
                    .as_array()
                    .and_then(|p| (p.len() == 2).then_some(p))
                    .ok_or_else(|| {
                        bad(format!(
                            "depth_json {name} level is not a [price, qty] pair"
                        ))
                    })?;
                let price = Decimal::from_str(
                    pair[0]
                        .as_str()
                        .ok_or_else(|| bad("depth_json price is not a string".into()))?,
                )
                .map_err(|e| bad(format!("depth_json price parse: {e}")))?;
                let qty = Decimal::from_str(
                    pair[1]
                        .as_str()
                        .ok_or_else(|| bad("depth_json qty is not a string".into()))?,
                )
                .map_err(|e| bad(format!("depth_json qty parse: {e}")))?;
                out.push((Price::new(price), Quantity::new(qty)));
            }
            Ok(out)
        };
    let bids = level(
        value
            .get("bids")
            .ok_or_else(|| bad("depth_json has no bids".into()))?,
        "bids",
    )?;
    let asks = level(
        value
            .get("asks")
            .ok_or_else(|| bad("depth_json has no asks".into()))?,
        "asks",
    )?;
    Ok(DepthSnapshot {
        as_of: UnixNanos::from_u64(0),
        bids,
        asks,
    })
}

/// Writes `depth` order-book snapshots, in the order given, as one tick file at `path`.
///
/// Like [`write_tick_file`], this creates the parent directory and overwrites an existing
/// file; the reader sorts by `ts_event`, so the order here only decides ties.
pub fn write_depth_file(path: &Path, depth: &[DepthTick]) -> Result<(), TickStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut kinds = Vec::with_capacity(depth.len());
    let mut symbols = Vec::with_capacity(depth.len());
    let mut exchanges = Vec::with_capacity(depth.len());
    let mut ts_events = Vec::with_capacity(depth.len());
    let mut ts_inits = Vec::with_capacity(depth.len());
    let mut depth_jsons = Vec::with_capacity(depth.len());
    for tick in depth {
        kinds.push("depth");
        symbols.push(tick.instrument.symbol.clone());
        exchanges.push(exchange_str(tick.instrument.exchange));
        ts_events.push(tick.ts_event.as_u64() as i64);
        ts_inits.push(Some(tick.ts_event.as_u64() as i64));
        depth_jsons.push(Some(depth_json(&tick.snapshot)));
    }

    let ints = |values: Vec<i64>| -> ArrayRef { std::sync::Arc::new(Int64Array::from(values)) };
    let opt_ints =
        |values: Vec<Option<i64>>| -> ArrayRef { std::sync::Arc::new(Int64Array::from(values)) };
    let opt_owned = |values: Vec<Option<String>>| -> ArrayRef {
        std::sync::Arc::new(StringArray::from_iter(values))
    };
    let strings =
        |values: Vec<&str>| -> ArrayRef { std::sync::Arc::new(StringArray::from(values)) };

    let empty_floats: Vec<Option<f64>> = vec![None; depth.len()];
    let floats =
        |values: Vec<Option<f64>>| -> ArrayRef { std::sync::Arc::new(Float64Array::from(values)) };
    let null_strings = || -> ArrayRef {
        std::sync::Arc::new(StringArray::from_iter(std::iter::repeat_n(
            None::<&str>,
            depth.len(),
        )))
    };

    let columns: Vec<ArrayRef> = vec![
        strings(kinds),
        std::sync::Arc::new(StringArray::from_iter_values(symbols)),
        strings(exchanges),
        ints(ts_events),
        opt_ints(ts_inits),
        floats(empty_floats.clone()),
        floats(empty_floats.clone()),
        floats(empty_floats.clone()),
        floats(empty_floats.clone()),
        floats(empty_floats.clone()),
        floats(empty_floats.clone()),
        null_strings(),
        null_strings(),
        opt_owned(depth_jsons),
    ];
    let schema = std::sync::Arc::new(tick_schema());
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    let mut writer = ArrowWriter::try_new(File::create(path)?, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

/// The stored spelling of an exchange (`NSE`/`BSE`/`NFO`/`MCX`/`CDS`), via `Display`.
///
/// `tantra_core::Exchange` has five variants, so a `&'static str` per
/// variant avoids an allocation in the hot write path.
fn exchange_str(exchange: Exchange) -> &'static str {
    match exchange {
        Exchange::Nse => "NSE",
        Exchange::Bse => "BSE",
        Exchange::Nfo => "NFO",
        Exchange::Mcx => "MCX",
        Exchange::Cds => "CDS",
    }
}

/// The stored spelling of an aggressor side: its serde name, so the column and the wire agree
/// and a new variant stored by a later build is still readable text.
fn aggressor_name(side: AggressorSide) -> String {
    serde_json::to_value(side)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "no_aggressor".to_string())
}

/// Parses the stored aggressor spelling through the wire type, so a name this build does not
/// know is `None` rather than silently coerced.
fn parse_aggressor(value: &str) -> Option<AggressorSide> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

fn read_file(
    path: &Path,
    quotes: &mut Vec<QuoteTick>,
    trades: &mut Vec<TradeTick>,
    depth: &mut Vec<DepthTick>,
) -> Result<(), TickStoreError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for batch in reader {
        append_batch(&batch?, &name, quotes, trades, depth)?;
    }
    Ok(())
}

fn append_batch(
    batch: &RecordBatch,
    file: &str,
    quotes: &mut Vec<QuoteTick>,
    trades: &mut Vec<TradeTick>,
    depth: &mut Vec<DepthTick>,
) -> Result<(), TickStoreError> {
    let schema = batch.schema();
    let column = |name: &'static str| -> Result<ArrayRef, TickStoreError> {
        let index = schema
            .index_of(name)
            .map_err(|_| TickStoreError::MissingColumn(name))?;
        Ok(batch.column(index).clone())
    };
    let utf8 = |name: &'static str| -> Result<StringArray, TickStoreError> {
        let array = column(name)?;
        match array.data_type() {
            DataType::Utf8 => Ok(array
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("checked Utf8")
                .clone()),
            got => Err(TickStoreError::WrongType {
                name,
                got: got.clone(),
            }),
        }
    };
    let nanos = |name: &'static str| -> Result<Int64Array, TickStoreError> {
        let array = column(name)?;
        match array.data_type() {
            DataType::Int64 => Ok(array
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("checked Int64")
                .clone()),
            got => Err(TickStoreError::WrongType {
                name,
                got: got.clone(),
            }),
        }
    };
    let floats = |name: &'static str| -> Result<Float64Array, TickStoreError> {
        let array = column(name)?;
        match array.data_type() {
            DataType::Float64 => Ok(array
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("checked Float64")
                .clone()),
            got => Err(TickStoreError::WrongType {
                name,
                got: got.clone(),
            }),
        }
    };

    let kinds = utf8("kind")?;
    let symbols = utf8("symbol")?;
    let exchanges = utf8("exchange")?;
    let ts_events = nanos("ts_event")?;
    let ts_inits = nanos("ts_init")?;
    let bid_prices = floats("bid_price")?;
    let ask_prices = floats("ask_price")?;
    let bid_sizes = floats("bid_size")?;
    let ask_sizes = floats("ask_size")?;
    let prices = floats("price")?;
    let sizes = floats("size")?;
    let aggressors = utf8("aggressor")?;
    let trade_ids = utf8("trade_id")?;

    // The `depth_json` column only exists in files written by [`write_depth_file`]; older
    // quote/trade-only files lack it, so its absence is tolerated (a `depth` row in such a
    // file is an error, caught per-row below).
    let depth_jsons = match batch.schema().index_of("depth_json") {
        Ok(_) => Some(utf8("depth_json")?),
        Err(_) => None,
    };

    let bad = |row: usize, message: String| TickStoreError::BadRow {
        file: file.to_string(),
        row,
        message,
    };
    let required_f64 =
        |array: &Float64Array, row: usize, name: &str| -> Result<f64, TickStoreError> {
            if array.is_null(row) {
                Err(bad(row, format!("{name} is null on a tick that needs it")))
            } else {
                Ok(array.value(row))
            }
        };

    for row in 0..batch.num_rows() {
        let exchange = Exchange::from_str(exchanges.value(row)).map_err(|_| {
            bad(
                row,
                format!("{} is not a known exchange", exchanges.value(row)),
            )
        })?;
        let instrument = InstrumentId::new(exchange, symbols.value(row).to_string());
        let wire_instrument = serde_json::to_value(&instrument).expect("an instrument serializes");
        let ts_event = UnixNanos::from_u64(ts_events.value(row) as u64);
        let ts_init = if ts_inits.is_null(row) {
            ts_event
        } else {
            UnixNanos::from_u64(ts_inits.value(row) as u64)
        };
        let wire_event = serde_json::to_value(ts_event).expect("a timestamp serializes");
        let wire_init = serde_json::to_value(ts_init).expect("a timestamp serializes");
        match kinds.value(row) {
            // Through the wire type, like the NDJSON loader, so its invariants are checked once
            // in `honba-messages` and a bad row is an error, never a panic.
            "quote" => {
                let value = serde_json::json!({
                    "instrument_id": wire_instrument,
                    "bid_price": required_f64(&bid_prices, row, "bid_price")?,
                    "ask_price": required_f64(&ask_prices, row, "ask_price")?,
                    "bid_size": required_f64(&bid_sizes, row, "bid_size")?,
                    "ask_size": required_f64(&ask_sizes, row, "ask_size")?,
                    "ts_event": wire_event,
                    "ts_init": wire_init,
                });
                let tick: QuoteTick = serde_json::from_value(value)
                    .map_err(|e| bad(row, format!("invalid quote: {e}")))?;
                quotes.push(tick);
            }
            "trade" => {
                let aggressor = if aggressors.is_null(row) {
                    return Err(bad(row, "aggressor is null on a trade".to_string()));
                } else {
                    parse_aggressor(aggressors.value(row)).ok_or_else(|| {
                        bad(
                            row,
                            format!("unknown aggressor {:?}", aggressors.value(row)),
                        )
                    })?
                };
                let trade_id = if trade_ids.is_null(row) {
                    return Err(bad(row, "trade_id is null on a trade".to_string()));
                } else {
                    TradeId::new(trade_ids.value(row))
                };
                let value = serde_json::json!({
                    "instrument_id": wire_instrument,
                    "price": required_f64(&prices, row, "price")?,
                    "size": required_f64(&sizes, row, "size")?,
                    "aggressor_side": serde_json::to_value(aggressor).expect("an aggressor serializes"),
                    "trade_id": trade_id.as_str(),
                    "ts_event": wire_event,
                    "ts_init": wire_init,
                });
                let tick: TradeTick = serde_json::from_value(value)
                    .map_err(|e| bad(row, format!("invalid trade: {e}")))?;
                trades.push(tick);
            }
            "depth" => {
                let depth_jsons = depth_jsons.as_ref().ok_or_else(|| {
                    bad(
                        row,
                        "depth_json column missing on a depth tick row".to_string(),
                    )
                })?;
                let raw = if depth_jsons.is_null(row) {
                    return Err(bad(row, "depth_json is null on a depth tick".to_string()));
                } else {
                    depth_jsons.value(row)
                };
                let mut snapshot = parse_depth_json(raw, file, row)?;
                snapshot.as_of = ts_event;
                depth.push(DepthTick {
                    instrument,
                    ts_event,
                    snapshot,
                });
            }
            other => return Err(bad(row, format!("unknown kind {other:?}"))),
        }
    }
    Ok(())
}
