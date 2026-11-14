//! Delivery-percentage projection reader (gap `tantra-data` 0018).
//!
//! `tantra-data` publishes one Parquet file per trade date under
//! `{root}/catalog/delivery/<trade_date>.parquet` with columns `trade_date`
//! (date32), `exchange` (string), `instrument` (string), `traded_quantity`
//! (int64), `deliverable_quantity` (int64) and `delivery_pct`
//! (decimal128(18,4)). This module decodes those files into owned
//! [`DeliveryProjection`] records with exact `Decimal` percentages.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, Date32Array, Decimal128Array, Int64Array, StringArray};
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

/// Scale of the decimal128(18,4) `delivery_pct` column in the projection.
const DECIMAL_SCALE: u32 = 4;

/// Errors that can occur while reading delivery projections.
#[derive(Debug, Error)]
pub enum DeliveryProjectionError {
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

/// One row of the delivery projection.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryProjection {
    /// The trade date (partition key).
    pub trade_date: NaiveDate,
    /// Exchange that reported the delivery, e.g. `NSE`.
    pub exchange: String,
    /// Canonical instrument id string, e.g. `NSE:RELIANCE-EQ`.
    pub instrument: String,
    /// Total quantity traded that day.
    pub traded_quantity: i64,
    /// Quantity actually delivered on the settlement date.
    pub deliverable_quantity: i64,
    /// Deliverable quantity as a percentage of traded quantity (scale 4).
    pub delivery_pct: Decimal,
}

/// Reader over the `{root}/catalog/delivery` projection directory.
#[derive(Clone)]
pub struct DeliveryProjectionReader {
    dir: PathBuf,
}

impl DeliveryProjectionReader {
    /// Creates a reader rooted at `root`, the `tantra-data` catalog root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            dir: root.as_ref().join("catalog").join("delivery"),
        }
    }

    /// Reads every `*.parquet` file in the directory, sorted by name.
    ///
    /// Returns an empty vector when the directory does not exist.
    pub fn read(&self) -> Result<Vec<DeliveryProjection>, DeliveryProjectionError> {
        if !self.dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "parquet"))
            .collect();
        files.sort();

        let mut deliveries = Vec::new();
        for file in files {
            let input = File::open(file)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(input)?;
            for batch in builder.build()? {
                deliveries.extend(delivery_projections_of(&batch?)?);
            }
        }
        Ok(deliveries)
    }
}

/// Decodes one record batch into owned [`DeliveryProjection`] rows.
fn delivery_projections_of(
    batch: &RecordBatch,
) -> Result<Vec<DeliveryProjection>, DeliveryProjectionError> {
    let schema = batch.schema();
    let index = |name: &'static str| -> Result<usize, DeliveryProjectionError> {
        schema
            .index_of(name)
            .map_err(|_| DeliveryProjectionError::MissingColumn(name))
    };
    let i_trade_date = index("trade_date")?;
    let i_exchange = index("exchange")?;
    let i_instrument = index("instrument")?;
    let i_traded_quantity = index("traded_quantity")?;
    let i_deliverable_quantity = index("deliverable_quantity")?;
    let i_delivery_pct = index("delivery_pct")?;

    let trade_dates = date32_array(batch.column(i_trade_date), "trade_date")?;
    let exchanges = string_array(batch.column(i_exchange), "exchange")?;
    let instruments = string_array(batch.column(i_instrument), "instrument")?;
    let traded_quantities = int64_array(batch.column(i_traded_quantity), "traded_quantity")?;
    let deliverable_quantities =
        int64_array(batch.column(i_deliverable_quantity), "deliverable_quantity")?;
    let delivery_pcts = decimal128_array(batch.column(i_delivery_pct), "delivery_pct")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let days = trade_dates.value(row);
        let trade_date = date32_to_naive_date(days).ok_or_else(|| {
            arrow::error::ArrowError::InvalidArgumentError(format!(
                "invalid trade_date value {days}"
            ))
        })?;
        rows.push(DeliveryProjection {
            trade_date,
            exchange: exchanges.value(row).to_string(),
            instrument: instruments.value(row).to_string(),
            traded_quantity: traded_quantities.value(row),
            deliverable_quantity: deliverable_quantities.value(row),
            delivery_pct: Decimal::try_from_i128_with_scale(
                delivery_pcts.value(row),
                DECIMAL_SCALE,
            )
            .map_err(|_| DeliveryProjectionError::BadDecimal {
                column: "delivery_pct",
            })?,
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
) -> Result<&'a Date32Array, DeliveryProjectionError> {
    column
        .as_any()
        .downcast_ref::<Date32Array>()
        .ok_or(DeliveryProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`StringArray`], failing on a wrong type.
fn string_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a StringArray, DeliveryProjectionError> {
    column
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or(DeliveryProjectionError::MissingColumn(name))
}

/// Downcasts a column to an [`Int64Array`], failing on a wrong type.
fn int64_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Int64Array, DeliveryProjectionError> {
    column
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or(DeliveryProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`Decimal128Array`], failing on a wrong type.
fn decimal128_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Decimal128Array, DeliveryProjectionError> {
    column
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .ok_or(DeliveryProjectionError::MissingColumn(name))
}
