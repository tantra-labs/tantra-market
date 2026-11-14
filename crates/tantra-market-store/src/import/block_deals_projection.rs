//! Block/bulk-deal projection reader (gap `tantra-data` 0005).
//!
//! `tantra-data` publishes one Parquet file per trade date under
//! `{root}/catalog/block_deals/<trade_date>.parquet` with columns `trade_date`
//! (date32), `exchange` (string), `instrument` (string), `security_name`
//! (string), `client_name` (string), `side` (string), `category` (string),
//! `quantity` (decimal128(18,4)), `price` (decimal128(18,4)) and `remarks`
//! (string, nullable). This module decodes those files into owned
//! [`BlockDealProjection`] records with exact `Decimal` amounts.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, Date32Array, Decimal128Array, StringArray};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rust_decimal::Decimal;
use thiserror::Error;

/// Days from the proleptic Gregorian CE epoch (0001-01-01) to the Unix epoch
/// (1970-01-01). Arrow's date32 type is measured from the Unix epoch while
/// [`NaiveDate::from_num_days_from_ce_opt`] is measured from the CE epoch
/// with day 1 = 0001-01-01, hence the +1.
const UNIX_EPOCH_DAYS_FROM_CE: i64 = 719_163;

/// Scale of the decimal128(18,4) quantity/price columns in the projection.
const DECIMAL_SCALE: u32 = 4;

/// Errors that can occur while reading block-deal projections.
#[derive(Debug, Error)]
pub enum BlockDealsProjectionError {
    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A parquet reader error occurred.
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// An Arrow array conversion error occurred.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// A required column is missing or of an unexpected type.
    #[error("missing column: {0}")]
    MissingColumn(&'static str),
    /// A decimal column held an unrepresentable value.
    #[error("failed to decode decimal in column {column}")]
    BadDecimal {
        /// Name of the column whose value could not be decoded.
        column: &'static str,
    },
}

/// One row of the block/bulk-deal projection.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockDealProjection {
    /// The trade date (partition key).
    pub trade_date: NaiveDate,
    /// Exchange that published the disclosure, e.g. `NSE`.
    pub exchange: String,
    /// Canonical instrument id string, e.g. `NSE:RELIANCE-EQ`.
    pub instrument: String,
    /// Human-readable security name from the disclosure.
    pub security_name: String,
    /// Client/counterparty name from the disclosure.
    pub client_name: String,
    /// Deal side: `BUY` or `SELL`.
    pub side: String,
    /// Deal category: `block` or `bulk`.
    pub category: String,
    /// Traded quantity (scale 4).
    pub quantity: Decimal,
    /// Trade / weighted-average price (scale 4).
    pub price: Decimal,
    /// Optional remarks column (present on NSE's bulk CSV).
    pub remarks: Option<String>,
}

/// Reader over the `{root}/catalog/block_deals` projection directory.
pub struct BlockDealsProjectionReader {
    dir: PathBuf,
}

impl BlockDealsProjectionReader {
    /// Creates a reader rooted at `root`, the `tantra-data` catalog root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            dir: root.as_ref().join("catalog").join("block_deals"),
        }
    }

    /// Reads every `*.parquet` file in the directory, sorted by name.
    ///
    /// Returns an empty vector when the directory does not exist.
    pub fn read(&self) -> Result<Vec<BlockDealProjection>, BlockDealsProjectionError> {
        if !self.dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "parquet"))
            .collect();
        files.sort();

        let mut deals = Vec::new();
        for file in files {
            let input = File::open(file)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(input)?;
            for batch in builder.build()? {
                deals.extend(block_deal_projections_of(&batch?)?);
            }
        }
        Ok(deals)
    }
}

/// Decodes one record batch into owned [`BlockDealProjection`] rows.
fn block_deal_projections_of(
    batch: &RecordBatch,
) -> Result<Vec<BlockDealProjection>, BlockDealsProjectionError> {
    let schema = batch.schema();
    let index = |name: &'static str| -> Result<usize, BlockDealsProjectionError> {
        schema
            .index_of(name)
            .map_err(|_| BlockDealsProjectionError::MissingColumn(name))
    };
    let i_trade_date = index("trade_date")?;
    let i_exchange = index("exchange")?;
    let i_instrument = index("instrument")?;
    let i_security_name = index("security_name")?;
    let i_client_name = index("client_name")?;
    let i_side = index("side")?;
    let i_category = index("category")?;
    let i_quantity = index("quantity")?;
    let i_price = index("price")?;
    let i_remarks = index("remarks")?;

    let trade_dates = date32_array(batch.column(i_trade_date), "trade_date")?;
    let exchanges = string_array(batch.column(i_exchange), "exchange")?;
    let instruments = string_array(batch.column(i_instrument), "instrument")?;
    let security_names = string_array(batch.column(i_security_name), "security_name")?;
    let client_names = string_array(batch.column(i_client_name), "client_name")?;
    let sides = string_array(batch.column(i_side), "side")?;
    let categories = string_array(batch.column(i_category), "category")?;
    let quantities = decimal128_array(batch.column(i_quantity), "quantity")?;
    let prices = decimal128_array(batch.column(i_price), "price")?;
    let remarks = string_array(batch.column(i_remarks), "remarks")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let days = trade_dates.value(row);
        let trade_date = date32_to_naive_date(days).ok_or_else(|| {
            arrow::error::ArrowError::InvalidArgumentError(format!(
                "invalid trade_date value {days}"
            ))
        })?;
        rows.push(BlockDealProjection {
            trade_date,
            exchange: exchanges.value(row).to_string(),
            instrument: instruments.value(row).to_string(),
            security_name: security_names.value(row).to_string(),
            client_name: client_names.value(row).to_string(),
            side: sides.value(row).to_string(),
            category: categories.value(row).to_string(),
            quantity: Decimal::try_from_i128_with_scale(quantities.value(row), DECIMAL_SCALE)
                .map_err(|_| BlockDealsProjectionError::BadDecimal { column: "quantity" })?,
            price: Decimal::try_from_i128_with_scale(prices.value(row), DECIMAL_SCALE)
                .map_err(|_| BlockDealsProjectionError::BadDecimal { column: "price" })?,
            remarks: if remarks.is_null(row) {
                None
            } else {
                Some(remarks.value(row).to_string())
            },
        });
    }
    Ok(rows)
}

/// Converts an Arrow date32 day count (days since 1970-01-01) to a [`NaiveDate`].
fn date32_to_naive_date(days_since_unix_epoch: i32) -> Option<NaiveDate> {
    let days_from_ce = i64::from(days_since_unix_epoch) + UNIX_EPOCH_DAYS_FROM_CE;
    NaiveDate::from_num_days_from_ce_opt(i32::try_from(days_from_ce).ok()?)
}

/// Downcasts a column to a [`Date32Array`], failing on a wrong type.
fn date32_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Date32Array, BlockDealsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Date32Array>()
        .ok_or(BlockDealsProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`StringArray`], failing on a wrong type.
fn string_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a StringArray, BlockDealsProjectionError> {
    column
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or(BlockDealsProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`Decimal128Array`], failing on a wrong type.
fn decimal128_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Decimal128Array, BlockDealsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .ok_or(BlockDealsProjectionError::MissingColumn(name))
}
