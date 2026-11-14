//! Point-in-time corporate-actions projection reader (gap `tantra-data` 0008).
//!
//! `tantra-data` publishes one Parquet file per ex-date under
//! `{root}/catalog/corporate_actions/<ex_date>.parquet` with columns `ex_date`
//! (date32), `instrument` (string), `action_type` (string), `old_shares`
//! (int32, nullable), `new_shares` (int32, nullable), `per_share`
//! (decimal128(18,4), nullable), `issue_price` (decimal128(18,4), nullable)
//! and `currency` (string). This module decodes those files into owned
//! [`CorporateActionProjection`] records with exact `Decimal` amounts.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, Date32Array, Decimal128Array, Int32Array, StringArray};
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

/// Scale of the decimal128(18,4) share-price columns in the projection.
const DECIMAL_SCALE: u32 = 4;

/// Errors that can occur while reading corporate-actions projections.
#[derive(Debug, Error)]
pub enum CorporateActionsProjectionError {
    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A parquet reader error occurred.
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// An Arrow array conversion error occurred.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// A decimal conversion error occurred.
    #[error("decimal error: {0}")]
    Decimal(#[from] rust_decimal::Error),
    /// A required column is missing or of an unexpected type.
    #[error("missing column: {0}")]
    MissingColumn(&'static str),
}

/// One row of the corporate-actions projection.
#[derive(Debug, Clone, PartialEq)]
pub struct CorporateActionProjection {
    /// The ex-date (point-in-time partition key).
    pub ex_date: NaiveDate,
    /// Canonical instrument id string, e.g. `NSE:RELIANCE-EQ`.
    pub instrument: String,
    /// Action type: `split`, `bonus`, `dividend` or `rights`.
    pub action_type: String,
    /// Shares held before the action, when disclosed.
    pub old_shares: Option<i32>,
    /// Shares held after the action, when disclosed.
    pub new_shares: Option<i32>,
    /// Amount per share, e.g. dividend per share (scale 4), when applicable.
    pub per_share: Option<Decimal>,
    /// Issue price for a rights issue, when applicable.
    pub issue_price: Option<Decimal>,
    /// ISO currency code of the amounts, `INR` by default.
    pub currency: String,
}

/// Reader over the `{root}/catalog/corporate_actions` projection directory.
pub struct CorporateActionsProjectionReader {
    dir: PathBuf,
}

impl CorporateActionsProjectionReader {
    /// Creates a reader rooted at `root`, the `tantra-data` catalog root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            dir: root.as_ref().join("catalog").join("corporate_actions"),
        }
    }

    /// Reads every `*.parquet` file in the directory, sorted by name.
    ///
    /// Returns an empty vector when the directory does not exist.
    pub fn read(&self) -> Result<Vec<CorporateActionProjection>, CorporateActionsProjectionError> {
        if !self.dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "parquet"))
            .collect();
        files.sort();

        let mut actions = Vec::new();
        for file in files {
            let input = File::open(file)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(input)?;
            for batch in builder.build()? {
                actions.extend(corporate_action_projections_of(&batch?)?);
            }
        }
        Ok(actions)
    }
}

/// Decodes one record batch into owned [`CorporateActionProjection`] rows.
fn corporate_action_projections_of(
    batch: &RecordBatch,
) -> Result<Vec<CorporateActionProjection>, CorporateActionsProjectionError> {
    let schema = batch.schema();
    let index = |name: &'static str| -> Result<usize, CorporateActionsProjectionError> {
        schema
            .index_of(name)
            .map_err(|_| CorporateActionsProjectionError::MissingColumn(name))
    };
    let i_ex_date = index("ex_date")?;
    let i_instrument = index("instrument")?;
    let i_action_type = index("action_type")?;
    let i_old_shares = index("old_shares")?;
    let i_new_shares = index("new_shares")?;
    let i_per_share = index("per_share")?;
    let i_issue_price = index("issue_price")?;
    let i_currency = index("currency")?;

    let ex_dates = date32_array(batch.column(i_ex_date), "ex_date")?;
    let instruments = string_array(batch.column(i_instrument), "instrument")?;
    let action_types = string_array(batch.column(i_action_type), "action_type")?;
    let old_shares = int32_array(batch.column(i_old_shares), "old_shares")?;
    let new_shares = int32_array(batch.column(i_new_shares), "new_shares")?;
    let per_share = decimal128_array(batch.column(i_per_share), "per_share")?;
    let issue_price = decimal128_array(batch.column(i_issue_price), "issue_price")?;
    let currencies = string_array(batch.column(i_currency), "currency")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let days = ex_dates.value(row);
        let ex_date = date32_to_naive_date(days).ok_or_else(|| {
            arrow::error::ArrowError::InvalidArgumentError(format!("invalid ex_date value {days}"))
        })?;
        rows.push(CorporateActionProjection {
            ex_date,
            instrument: instruments.value(row).to_string(),
            action_type: action_types.value(row).to_string(),
            old_shares: if old_shares.is_null(row) {
                None
            } else {
                Some(old_shares.value(row))
            },
            new_shares: if new_shares.is_null(row) {
                None
            } else {
                Some(new_shares.value(row))
            },
            per_share: if per_share.is_null(row) {
                None
            } else {
                Some(Decimal::try_from_i128_with_scale(
                    per_share.value(row),
                    DECIMAL_SCALE,
                )?)
            },
            issue_price: if issue_price.is_null(row) {
                None
            } else {
                Some(Decimal::try_from_i128_with_scale(
                    issue_price.value(row),
                    DECIMAL_SCALE,
                )?)
            },
            currency: currencies.value(row).to_string(),
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
) -> Result<&'a Date32Array, CorporateActionsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Date32Array>()
        .ok_or(CorporateActionsProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`StringArray`], failing on a wrong type.
fn string_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a StringArray, CorporateActionsProjectionError> {
    column
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or(CorporateActionsProjectionError::MissingColumn(name))
}

/// Downcasts a column to an [`Int32Array`], failing on a wrong type.
fn int32_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Int32Array, CorporateActionsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Int32Array>()
        .ok_or(CorporateActionsProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`Decimal128Array`], failing on a wrong type.
fn decimal128_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Decimal128Array, CorporateActionsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .ok_or(CorporateActionsProjectionError::MissingColumn(name))
}
