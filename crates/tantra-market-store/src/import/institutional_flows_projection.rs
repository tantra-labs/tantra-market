//! Institutional-flows projection reader (gap `tantra-data` 0017).
//!
//! `tantra-data` publishes one Parquet file per trade date under
//! `{root}/catalog/institutional_flows/<trade_date>.parquet` with columns
//! `trade_date` (date32), `participant` (string), `segment` (string),
//! `gross_buy` (decimal128(18,4)), `gross_sell` (decimal128(18,4)), `net`
//! (decimal128(18,4)) and `source` (string). This module decodes those files
//! into owned [`InstitutionalFlowProjection`] records with exact `Decimal`
//! amounts.

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

/// Scale of the decimal128(18,4) amount columns in the projection.
const DECIMAL_SCALE: u32 = 4;

/// Errors that can occur while reading institutional-flows projections.
#[derive(Debug, Error)]
pub enum InstitutionalFlowsProjectionError {
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

/// One row of the institutional-flows projection.
#[derive(Debug, Clone, PartialEq)]
pub struct InstitutionalFlowProjection {
    /// The trade date (partition key).
    pub trade_date: NaiveDate,
    /// Participating institution, e.g. `FII` or `DII`.
    pub participant: String,
    /// Market segment, e.g. `cash` or `derivatives`.
    pub segment: String,
    /// Gross purchase amount (scale 4).
    pub gross_buy: Decimal,
    /// Gross sale amount (scale 4).
    pub gross_sell: Decimal,
    /// Net amount, `gross_buy - gross_sell` (scale 4).
    pub net: Decimal,
    /// Publisher / feed name the flow numbers came from.
    pub source: String,
}

/// Reader over the `{root}/catalog/institutional_flows` projection directory.
#[derive(Clone)]
pub struct InstitutionalFlowsProjectionReader {
    dir: PathBuf,
}

impl InstitutionalFlowsProjectionReader {
    /// Creates a reader rooted at `root`, the `tantra-data` catalog root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            dir: root.as_ref().join("catalog").join("institutional_flows"),
        }
    }

    /// Reads every `*.parquet` file in the directory, sorted by name.
    ///
    /// Returns an empty vector when the directory does not exist.
    pub fn read(
        &self,
    ) -> Result<Vec<InstitutionalFlowProjection>, InstitutionalFlowsProjectionError> {
        if !self.dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "parquet"))
            .collect();
        files.sort();

        let mut flows = Vec::new();
        for file in files {
            let input = File::open(file)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(input)?;
            for batch in builder.build()? {
                flows.extend(institutional_flow_projections_of(&batch?)?);
            }
        }
        Ok(flows)
    }
}

/// Decodes one record batch into owned [`InstitutionalFlowProjection`] rows.
fn institutional_flow_projections_of(
    batch: &RecordBatch,
) -> Result<Vec<InstitutionalFlowProjection>, InstitutionalFlowsProjectionError> {
    let schema = batch.schema();
    let index = |name: &'static str| -> Result<usize, InstitutionalFlowsProjectionError> {
        schema
            .index_of(name)
            .map_err(|_| InstitutionalFlowsProjectionError::MissingColumn(name))
    };
    let i_trade_date = index("trade_date")?;
    let i_participant = index("participant")?;
    let i_segment = index("segment")?;
    let i_gross_buy = index("gross_buy")?;
    let i_gross_sell = index("gross_sell")?;
    let i_net = index("net")?;
    let i_source = index("source")?;

    let trade_dates = date32_array(batch.column(i_trade_date), "trade_date")?;
    let participants = string_array(batch.column(i_participant), "participant")?;
    let segments = string_array(batch.column(i_segment), "segment")?;
    let gross_buys = decimal128_array(batch.column(i_gross_buy), "gross_buy")?;
    let gross_sells = decimal128_array(batch.column(i_gross_sell), "gross_sell")?;
    let nets = decimal128_array(batch.column(i_net), "net")?;
    let sources = string_array(batch.column(i_source), "source")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let days = trade_dates.value(row);
        let trade_date = date32_to_naive_date(days).ok_or_else(|| {
            arrow::error::ArrowError::InvalidArgumentError(format!(
                "invalid trade_date value {days}"
            ))
        })?;
        rows.push(InstitutionalFlowProjection {
            trade_date,
            participant: participants.value(row).to_string(),
            segment: segments.value(row).to_string(),
            gross_buy: Decimal::try_from_i128_with_scale(gross_buys.value(row), DECIMAL_SCALE)
                .map_err(|_| InstitutionalFlowsProjectionError::BadDecimal {
                    column: "gross_buy",
                })?,
            gross_sell: Decimal::try_from_i128_with_scale(gross_sells.value(row), DECIMAL_SCALE)
                .map_err(|_| InstitutionalFlowsProjectionError::BadDecimal {
                    column: "gross_sell",
                })?,
            net: Decimal::try_from_i128_with_scale(nets.value(row), DECIMAL_SCALE)
                .map_err(|_| InstitutionalFlowsProjectionError::BadDecimal { column: "net" })?,
            source: sources.value(row).to_string(),
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
) -> Result<&'a Date32Array, InstitutionalFlowsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Date32Array>()
        .ok_or(InstitutionalFlowsProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`StringArray`], failing on a wrong type.
fn string_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a StringArray, InstitutionalFlowsProjectionError> {
    column
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or(InstitutionalFlowsProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`Decimal128Array`], failing on a wrong type.
fn decimal128_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Decimal128Array, InstitutionalFlowsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .ok_or(InstitutionalFlowsProjectionError::MissingColumn(name))
}
