//! Point-in-time filings projection reader (gap `tantra-data` 0007).
//!
//! `tantra-data` publishes filings under Lakehouse v3 (`fundamentals/filings` manifest/views) or
//! legacy `{root}/catalog/filings/<filing_date>.parquet` with columns `filing_date` (date32),
//! `instrument` (string), `filing_type` (string), `title` (string) and `source_url` (string,
//! nullable). This module decodes those files into owned [`FilingProjection`] records a Rust
//! consumer can filter point-in-time by filing date.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, Date32Array, StringArray};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use thiserror::Error;

use crate::manifest::{ManifestError, arrow_utf8_array, resolve_projection_files};

/// Days from the proleptic Gregorian CE epoch (0001-01-01) to the Unix epoch
/// (1970-01-01). Arrow's date32 type is measured from the Unix epoch while
/// [`NaiveDate::from_num_days_from_ce_opt`] is measured from the CE epoch
/// with day 1 = 0001-01-01, hence the +1.
const UNIX_EPOCH_DAYS_FROM_CE: i64 = 719_163;

/// Errors that can occur while reading filings projections.
#[derive(Debug, Error)]
pub enum FilingsProjectionError {
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
}

/// One row of the filings projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilingProjection {
    /// The filing date (point-in-time partition key).
    pub filing_date: NaiveDate,
    /// Canonical instrument id string, e.g. `NSE:RELIANCE-EQ`.
    pub instrument: String,
    /// Filing type: `corporate_action`, `shareholding`, `insider_trade`,
    /// `fund_holding`, `annual_report`, `quarterly_result` or `other`.
    pub filing_type: String,
    /// Filing title as published.
    pub title: String,
    /// Source URL of the filing, when known.
    pub source_url: Option<String>,
}

/// Reader over Lakehouse v3 `fundamentals/filings` or legacy `{root}/catalog/filings`.
#[derive(Clone)]
pub struct FilingsProjectionReader {
    root: PathBuf,
    reference: Option<String>,
    legacy_fallback: bool,
}

impl FilingsProjectionReader {
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

    /// Enables or disables legacy `{root}/catalog/filings` fallback (default `true`).
    #[must_use]
    pub fn with_legacy_fallback(mut self, enabled: bool) -> Self {
        self.legacy_fallback = enabled;
        self
    }

    /// Reads every matching `*.parquet` file, sorted by manifest/path order.
    ///
    /// Returns an empty vector when no files exist.
    pub fn read(&self) -> Result<Vec<FilingProjection>, FilingsProjectionError> {
        let files = resolve_projection_files(
            &self.root,
            "fundamentals",
            "filings",
            "filings",
            self.reference.as_deref(),
            self.legacy_fallback,
        )?;

        let mut projections = Vec::new();
        for file in files {
            let input = File::open(file)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(input)?;
            for batch in builder.build()? {
                projections.extend(filing_projections_of(&batch?)?);
            }
        }
        Ok(projections)
    }
}

/// Decodes one record batch into owned [`FilingProjection`] rows.
fn filing_projections_of(
    batch: &RecordBatch,
) -> Result<Vec<FilingProjection>, FilingsProjectionError> {
    let schema = batch.schema();
    let index = |name: &'static str| -> Result<usize, FilingsProjectionError> {
        schema
            .index_of(name)
            .map_err(|_| FilingsProjectionError::MissingColumn(name))
    };
    let i_filing_date = index("filing_date")?;
    let i_instrument = index("instrument")?;
    let i_filing_type = index("filing_type")?;
    let i_title = index("title")?;
    let i_source_url = index("source_url")?;

    let filing_dates = date32_array(batch.column(i_filing_date), "filing_date")?;
    let instruments = string_array(batch.column(i_instrument), "instrument")?;
    let filing_types = string_array(batch.column(i_filing_type), "filing_type")?;
    let titles = string_array(batch.column(i_title), "title")?;
    let source_urls = string_array(batch.column(i_source_url), "source_url")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let days = filing_dates.value(row);
        let filing_date = date32_to_naive_date(days).ok_or_else(|| {
            arrow::error::ArrowError::InvalidArgumentError(format!(
                "invalid filing_date value {days}"
            ))
        })?;
        rows.push(FilingProjection {
            filing_date,
            instrument: instruments.value(row).to_string(),
            filing_type: filing_types.value(row).to_string(),
            title: titles.value(row).to_string(),
            source_url: if source_urls.is_null(row) {
                None
            } else {
                Some(source_urls.value(row).to_string())
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
) -> Result<&'a Date32Array, FilingsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Date32Array>()
        .ok_or(FilingsProjectionError::MissingColumn(name))
}

/// Decodes a `Utf8` or `LargeUtf8` column to a [`StringArray`].
fn string_array(
    column: &dyn Array,
    name: &'static str,
) -> Result<StringArray, FilingsProjectionError> {
    arrow_utf8_array(column).map_err(|_| FilingsProjectionError::MissingColumn(name))
}
