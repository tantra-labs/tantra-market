//! Fundamentals / financial-ratios projection reader (gap `tantra-data` 0020).
//!
//! `tantra-data` publishes two projection datasets (under Lakehouse v3 `fundamentals/fundamentals`
//! and `fundamentals/fundamental_ratios` manifests/views or legacy `{root}/catalog/...`):
//! - `fundamentals`: one financial-statement line item per row: `filing_date` (date32),
//!   `instrument` (string), `statement_type` (string), `period_type` (string), `period_end_date`
//!   (date32), `line_item` (string), `value` (decimal128(18,4)) and `currency` (string).
//!   Point-in-time semantics: filter `filing_date <= as_of` so a backtest never observes an unfiled
//!   fact.
//! - `fundamental_ratios`: one ratio snapshot per `(instrument, as_of_date)`: `as_of_date` (date32),
//!   `instrument` (string), `pe`, `pb`, `ev_to_ebitda`, `dividend_yield`, `roce`, `roe`,
//!   `debt_to_equity`, `interest_coverage` (decimal128(18,4), nullable) and `source` (string).
//!
//! This module decodes those files into owned projections with exact `Decimal` values.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, Date32Array, Decimal128Array, StringArray};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::manifest::{ManifestError, arrow_utf8_array, resolve_projection_files};

/// Days from the proleptic Gregorian CE epoch (0001-01-01) to the Unix epoch
/// (1970-01-01). Arrow's date32 type is measured from the Unix epoch while
/// [`NaiveDate::from_num_days_from_ce_opt`] is measured from the CE epoch
/// with day 1 = 0001-01-01, hence the +1.
const UNIX_EPOCH_DAYS_FROM_CE: i64 = 719_163;

/// Scale of the decimal128(18,4) value columns in the projections.
const DECIMAL_SCALE: u32 = 4;

/// Errors that can occur while reading fundamentals projections.
#[derive(Debug, Error)]
pub enum FundamentalsProjectionError {
    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A parquet reader error occurred.
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// An Arrow array conversion error occurred.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// A Lakehouse v3 manifest error occurred.
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
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

/// One financial-statement line item of the fundamentals projection.
#[derive(Debug, Clone, PartialEq)]
pub struct FundamentalStatementProjection {
    /// The date the disclosure became public (partition key; as-of filter).
    pub filing_date: NaiveDate,
    /// Canonical instrument id string, e.g. `NSE:RELIANCE-EQ`.
    pub instrument: String,
    /// Statement family: `income_statement` | `balance_sheet` | `cash_flow`.
    pub statement_type: String,
    /// Period cadence: `annual` | `quarterly`.
    pub period_type: String,
    /// The fiscal period end date the line item belongs to.
    pub period_end_date: NaiveDate,
    /// Canonical snake_case metric name, e.g. `total_revenue`, `net_income`.
    pub line_item: String,
    /// Exact money value (scale 4), e.g. `10572190000000`.
    pub value: Decimal,
    /// Reporting currency, e.g. `INR`.
    pub currency: String,
}

/// One point-in-time valuation / profitability ratio snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct FundamentalRatiosProjection {
    /// The date the snapshot was taken (partition key).
    pub as_of_date: NaiveDate,
    /// Canonical instrument id string, e.g. `NSE:RELIANCE-EQ`.
    pub instrument: String,
    /// Price-to-earnings ratio, when reported.
    pub pe: Option<Decimal>,
    /// Price-to-book ratio, when reported.
    pub pb: Option<Decimal>,
    /// Enterprise value / EBITDA, when reported.
    pub ev_to_ebitda: Option<Decimal>,
    /// Dividend yield percent, when reported.
    pub dividend_yield: Option<Decimal>,
    /// Return on capital employed percent, when reported.
    pub roce: Option<Decimal>,
    /// Return on equity percent, when reported.
    pub roe: Option<Decimal>,
    /// Debt-to-equity ratio, when reported.
    pub debt_to_equity: Option<Decimal>,
    /// Interest coverage ratio, when reported.
    pub interest_coverage: Option<Decimal>,
    /// Publisher / feed the numbers came from.
    pub source: String,
}

/// Reader over Lakehouse v3 `fundamentals/{fundamentals,fundamental_ratios}` or legacy `{root}/catalog/...`.
#[derive(Clone)]
pub struct FundamentalsProjectionReader {
    root: PathBuf,
    reference: Option<String>,
    legacy_fallback: bool,
}

impl FundamentalsProjectionReader {
    /// Creates a reader rooted at `root`, the `tantra-data` lakehouse/catalog root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            reference: None,
            legacy_fallback: true,
        }
    }

    /// Pins reads to a specific Lakehouse v3 commit/tag/branch reference.
    #[must_use]
    pub fn with_ref(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    /// Enables or disables legacy `{root}/catalog/*` fallback (default `true`).
    #[must_use]
    pub fn with_legacy_fallback(mut self, enabled: bool) -> Self {
        self.legacy_fallback = enabled;
        self
    }

    /// Reads every financial-statement line item in the projection.
    ///
    /// Returns an empty vector when no files exist.
    pub fn read_statements(
        &self,
    ) -> Result<Vec<FundamentalStatementProjection>, FundamentalsProjectionError> {
        let files = resolve_projection_files(
            &self.root,
            "fundamentals",
            "fundamentals",
            "fundamentals",
            self.reference.as_deref(),
            self.legacy_fallback,
        )?;
        let mut statements = Vec::new();
        for file in files {
            statements.extend(decode_file(&file, statement_projections_of)?);
        }
        Ok(statements)
    }

    /// Reads every point-in-time ratio snapshot in the projection.
    ///
    /// Returns an empty vector when no files exist.
    pub fn read_ratios(
        &self,
    ) -> Result<Vec<FundamentalRatiosProjection>, FundamentalsProjectionError> {
        let files = resolve_projection_files(
            &self.root,
            "fundamentals",
            "fundamental_ratios",
            "fundamental_ratios",
            self.reference.as_deref(),
            self.legacy_fallback,
        )?;
        let mut ratios = Vec::new();
        for file in files {
            ratios.extend(decode_file(&file, ratios_projections_of)?);
        }
        Ok(ratios)
    }
}

/// Decodes every batch of one parquet file through `decode`.
fn decode_file<T>(
    file: &Path,
    decode: fn(&RecordBatch) -> Result<Vec<T>, FundamentalsProjectionError>,
) -> Result<Vec<T>, FundamentalsProjectionError> {
    let input = File::open(file)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(input)?;
    let mut rows = Vec::new();
    for batch in builder.build()? {
        rows.extend(decode(&batch?)?);
    }
    Ok(rows)
}

/// Decodes one record batch into owned [`FundamentalStatementProjection`] rows.
fn statement_projections_of(
    batch: &RecordBatch,
) -> Result<Vec<FundamentalStatementProjection>, FundamentalsProjectionError> {
    let schema = batch.schema();
    let index = |name: &'static str| -> Result<usize, FundamentalsProjectionError> {
        schema
            .index_of(name)
            .map_err(|_| FundamentalsProjectionError::MissingColumn(name))
    };
    let i_filing_date = index("filing_date")?;
    let i_instrument = index("instrument")?;
    let i_statement_type = index("statement_type")?;
    let i_period_type = index("period_type")?;
    let i_period_end_date = index("period_end_date")?;
    let i_line_item = index("line_item")?;
    let i_value = index("value")?;
    let i_currency = index("currency")?;

    let filing_dates = date32_array(batch.column(i_filing_date), "filing_date")?;
    let instruments = string_array(batch.column(i_instrument), "instrument")?;
    let statement_types = string_array(batch.column(i_statement_type), "statement_type")?;
    let period_types = string_array(batch.column(i_period_type), "period_type")?;
    let period_end_dates = date32_array(batch.column(i_period_end_date), "period_end_date")?;
    let line_items = string_array(batch.column(i_line_item), "line_item")?;
    let values = decimal128_array(batch.column(i_value), "value")?;
    let currencies = string_array(batch.column(i_currency), "currency")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        rows.push(FundamentalStatementProjection {
            filing_date: date_at(filing_dates, row, "filing_date")?,
            instrument: instruments.value(row).to_string(),
            statement_type: statement_types.value(row).to_string(),
            period_type: period_types.value(row).to_string(),
            period_end_date: date_at(period_end_dates, row, "period_end_date")?,
            line_item: line_items.value(row).to_string(),
            value: decimal_at(values, row, "value")?,
            currency: currencies.value(row).to_string(),
        });
    }
    Ok(rows)
}

/// Decodes one record batch into owned [`FundamentalRatiosProjection`] rows.
fn ratios_projections_of(
    batch: &RecordBatch,
) -> Result<Vec<FundamentalRatiosProjection>, FundamentalsProjectionError> {
    let schema = batch.schema();
    let index = |name: &'static str| -> Result<usize, FundamentalsProjectionError> {
        schema
            .index_of(name)
            .map_err(|_| FundamentalsProjectionError::MissingColumn(name))
    };
    let i_as_of_date = index("as_of_date")?;
    let i_instrument = index("instrument")?;
    let i_pe = index("pe")?;
    let i_pb = index("pb")?;
    let i_ev_to_ebitda = index("ev_to_ebitda")?;
    let i_dividend_yield = index("dividend_yield")?;
    let i_roce = index("roce")?;
    let i_roe = index("roe")?;
    let i_debt_to_equity = index("debt_to_equity")?;
    let i_interest_coverage = index("interest_coverage")?;
    let i_source = index("source")?;

    let as_of_dates = date32_array(batch.column(i_as_of_date), "as_of_date")?;
    let instruments = string_array(batch.column(i_instrument), "instrument")?;
    let pe = decimal128_array(batch.column(i_pe), "pe")?;
    let pb = decimal128_array(batch.column(i_pb), "pb")?;
    let ev_to_ebitda = decimal128_array(batch.column(i_ev_to_ebitda), "ev_to_ebitda")?;
    let dividend_yield = decimal128_array(batch.column(i_dividend_yield), "dividend_yield")?;
    let roce = decimal128_array(batch.column(i_roce), "roce")?;
    let roe = decimal128_array(batch.column(i_roe), "roe")?;
    let debt_to_equity = decimal128_array(batch.column(i_debt_to_equity), "debt_to_equity")?;
    let interest_coverage =
        decimal128_array(batch.column(i_interest_coverage), "interest_coverage")?;
    let sources = string_array(batch.column(i_source), "source")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        rows.push(FundamentalRatiosProjection {
            as_of_date: date_at(as_of_dates, row, "as_of_date")?,
            instrument: instruments.value(row).to_string(),
            pe: optional_decimal_at(pe, row, "pe")?,
            pb: optional_decimal_at(pb, row, "pb")?,
            ev_to_ebitda: optional_decimal_at(ev_to_ebitda, row, "ev_to_ebitda")?,
            dividend_yield: optional_decimal_at(dividend_yield, row, "dividend_yield")?,
            roce: optional_decimal_at(roce, row, "roce")?,
            roe: optional_decimal_at(roe, row, "roe")?,
            debt_to_equity: optional_decimal_at(debt_to_equity, row, "debt_to_equity")?,
            interest_coverage: optional_decimal_at(interest_coverage, row, "interest_coverage")?,
            source: sources.value(row).to_string(),
        });
    }
    Ok(rows)
}

/// Reads a non-null date32 column value as a [`NaiveDate`].
fn date_at(
    column: &Date32Array,
    row: usize,
    name: &'static str,
) -> Result<NaiveDate, FundamentalsProjectionError> {
    let days = column.value(row);
    date32_to_naive_date(days).ok_or_else(|| {
        FundamentalsProjectionError::Arrow(arrow::error::ArrowError::InvalidArgumentError(format!(
            "invalid {name} value {days}"
        )))
    })
}

/// Reads a non-null decimal128 column value as an exact [`Decimal`].
fn decimal_at(
    column: &Decimal128Array,
    row: usize,
    name: &'static str,
) -> Result<Decimal, FundamentalsProjectionError> {
    if column.is_null(row) {
        return Err(FundamentalsProjectionError::BadDecimal { column: name });
    }
    Decimal::try_from_i128_with_scale(column.value(row), DECIMAL_SCALE)
        .map_err(|_| FundamentalsProjectionError::BadDecimal { column: name })
}

/// Reads a nullable decimal128 column value as `Option<Decimal>`.
fn optional_decimal_at(
    column: &Decimal128Array,
    row: usize,
    name: &'static str,
) -> Result<Option<Decimal>, FundamentalsProjectionError> {
    if column.is_null(row) {
        return Ok(None);
    }
    Decimal::try_from_i128_with_scale(column.value(row), DECIMAL_SCALE)
        .map(Some)
        .map_err(|_| FundamentalsProjectionError::BadDecimal { column: name })
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
) -> Result<&'a Date32Array, FundamentalsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Date32Array>()
        .ok_or(FundamentalsProjectionError::MissingColumn(name))
}

/// Decodes a `Utf8` or `LargeUtf8` column to a [`StringArray`].
fn string_array(
    column: &dyn Array,
    name: &'static str,
) -> Result<StringArray, FundamentalsProjectionError> {
    arrow_utf8_array(column).map_err(|_| FundamentalsProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`Decimal128Array`], failing on a wrong type.
fn decimal128_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Decimal128Array, FundamentalsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .ok_or(FundamentalsProjectionError::MissingColumn(name))
}
