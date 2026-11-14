//! News projection reader (gap `tantra-data` 0006).
//!
//! `tantra-data` publishes one Parquet file per publication date under
//! `{root}/catalog/news/<published_date>.parquet` with columns `published_date`
//! (date32), `published_at` (string, RFC3339 UTC), `article_id` (string),
//! `title` (string), `source` (string), `url` (string), `summary` (string,
//! nullable) and `instruments` (string, comma-joined). This module decodes
//! those files into owned [`NewsProjection`] records a Rust consumer (e.g. the
//! runtime chat adapters) can serve to the `news_search` tool.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, Date32Array, StringArray};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use thiserror::Error;

/// Days from the proleptic Gregorian CE epoch (0001-01-01) to the Unix epoch
/// (1970-01-01). Arrow's date32 type is measured from the Unix epoch while
/// [`NaiveDate::from_num_days_from_ce_opt`] is measured from the CE epoch
/// with day 1 = 0001-01-01, hence the +1.
const UNIX_EPOCH_DAYS_FROM_CE: i64 = 719_163;

/// Errors that can occur while reading news projections.
#[derive(Debug, Error)]
pub enum NewsProjectionError {
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
    /// A `published_at` value is not a parseable RFC3339 timestamp.
    #[error("invalid RFC3339 timestamp {value:?} in column {column}")]
    BadTimestamp {
        /// The offending column name.
        column: &'static str,
        /// The unparseable timestamp string.
        value: String,
    },
}

/// One row of the news projection.
#[derive(Debug, Clone, PartialEq)]
pub struct NewsProjection {
    /// The publication date (partition key).
    pub published_date: NaiveDate,
    /// Exact publication instant, normalized to UTC.
    pub published_at: chrono::DateTime<chrono::Utc>,
    /// Stable article identifier.
    pub article_id: String,
    /// Article headline.
    pub title: String,
    /// Publisher / feed name (e.g. `moneycontrol`).
    pub source: String,
    /// Article URL.
    pub url: String,
    /// Article summary / description, when present.
    pub summary: Option<String>,
    /// Instruments the article mentions (canonical `InstrumentId`-style
    /// symbols, e.g. `NSE:RELIANCE-EQ`).
    pub instruments: Vec<String>,
}

/// Reader over the `{root}/catalog/news` projection directory.
pub struct NewsProjectionReader {
    dir: PathBuf,
}

impl NewsProjectionReader {
    /// Creates a reader rooted at `root`, the `tantra-data` catalog root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            dir: root.as_ref().join("catalog").join("news"),
        }
    }

    /// Reads every `*.parquet` file in the directory, sorted by name.
    ///
    /// Returns an empty vector when the directory does not exist.
    pub fn read(&self) -> Result<Vec<NewsProjection>, NewsProjectionError> {
        if !self.dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "parquet"))
            .collect();
        files.sort();

        let mut projections = Vec::new();
        for file in files {
            let input = File::open(file)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(input)?;
            for batch in builder.build()? {
                projections.extend(news_projections_of(&batch?)?);
            }
        }
        Ok(projections)
    }
}

/// Decodes one record batch into owned [`NewsProjection`] rows.
fn news_projections_of(batch: &RecordBatch) -> Result<Vec<NewsProjection>, NewsProjectionError> {
    let schema = batch.schema();
    let index = |name: &'static str| -> Result<usize, NewsProjectionError> {
        schema
            .index_of(name)
            .map_err(|_| NewsProjectionError::MissingColumn(name))
    };
    let i_published_date = index("published_date")?;
    let i_published_at = index("published_at")?;
    let i_article_id = index("article_id")?;
    let i_title = index("title")?;
    let i_source = index("source")?;
    let i_url = index("url")?;
    let i_summary = index("summary")?;
    let i_instruments = index("instruments")?;

    let published_dates = date32_array(batch.column(i_published_date), "published_date")?;
    let published_ats = string_array(batch.column(i_published_at), "published_at")?;
    let article_ids = string_array(batch.column(i_article_id), "article_id")?;
    let titles = string_array(batch.column(i_title), "title")?;
    let sources = string_array(batch.column(i_source), "source")?;
    let urls = string_array(batch.column(i_url), "url")?;
    let summaries = string_array(batch.column(i_summary), "summary")?;
    let instruments = string_array(batch.column(i_instruments), "instruments")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let days = published_dates.value(row);
        let published_date = date32_to_naive_date(days).ok_or_else(|| {
            arrow::error::ArrowError::InvalidArgumentError(format!(
                "invalid published_date value {days}"
            ))
        })?;
        let published_at = chrono::DateTime::parse_from_rfc3339(published_ats.value(row))
            .map_err(|_| NewsProjectionError::BadTimestamp {
                column: "published_at",
                value: published_ats.value(row).to_string(),
            })?
            .with_timezone(&chrono::Utc);
        let instruments = if instruments.value(row).is_empty() {
            Vec::new()
        } else {
            instruments
                .value(row)
                .split(',')
                .map(|symbol| symbol.to_string())
                .collect()
        };
        rows.push(NewsProjection {
            published_date,
            published_at,
            article_id: article_ids.value(row).to_string(),
            title: titles.value(row).to_string(),
            source: sources.value(row).to_string(),
            url: urls.value(row).to_string(),
            summary: if summaries.is_null(row) {
                None
            } else {
                Some(summaries.value(row).to_string())
            },
            instruments,
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
) -> Result<&'a Date32Array, NewsProjectionError> {
    column
        .as_any()
        .downcast_ref::<Date32Array>()
        .ok_or(NewsProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`StringArray`], failing on a wrong type.
fn string_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a StringArray, NewsProjectionError> {
    column
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or(NewsProjectionError::MissingColumn(name))
}
