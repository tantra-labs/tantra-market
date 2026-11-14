//! Mutual-fund / IPO projection readers (gaps `tantra-data` 0021 + 0022).
//!
//! `tantra-data` publishes five projection datasets:
//! - `{root}/catalog/mutual_funds/<as_of_date>.parquet` — scheme metadata per NAV-dump date.
//! - `{root}/catalog/mf_nav/<nav_date>.parquet` — per-scheme NAV / repurchase / sale prices.
//! - `{root}/catalog/mf_returns/<as_of_date>.parquet` — derived CAGR / expense ratio.
//! - `{root}/catalog/ipos/<as_of_date>.parquet` — IPO issue metadata (price band, lot, size).
//! - `{root}/catalog/ipo_subscriptions/<as_of_date>.parquet` — live subscription multiples.
//!
//! All money/ratio columns are exact `decimal128(18,4)`; nullable when the vendor did not
//! report the value. This module decodes them into owned projections with exact `Decimal`s.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, Date32Array, Decimal128Array, StringArray};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rust_decimal::Decimal;
use thiserror::Error;

/// Days from the proleptic Gregorian CE epoch (0001-01-01) to the Unix epoch (1970-01-01).
const UNIX_EPOCH_DAYS_FROM_CE: i64 = 719_163;

/// Scale of the decimal128(18,4) value columns in the projections.
const DECIMAL_SCALE: u32 = 4;

/// Errors that can occur while reading the fund/IPO projections.
#[derive(Debug, Error)]
pub enum FundsIposProjectionError {
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

/// One mutual-fund scheme's metadata as of a NAV-dump date.
#[derive(Debug, Clone, PartialEq)]
pub struct MutualFundMetaProjection {
    /// The NAV-dump date the scheme was observed in.
    pub as_of_date: NaiveDate,
    /// AMFI scheme code.
    pub scheme_code: String,
    /// Scheme name.
    pub scheme_name: String,
    /// Asset management company.
    pub amc: String,
    /// Scheme category (e.g. `Large Cap`).
    pub category: String,
    /// Growth-plan ISIN, when present.
    pub isin_growth: Option<String>,
    /// Dividend-reinvestment ISIN, when present.
    pub isin_div_reinvestment: Option<String>,
    /// Plan label (e.g. `Growth` / `IDCW`).
    pub plan: String,
    /// Option label.
    pub option: String,
}

/// One scheme's NAV observation.
#[derive(Debug, Clone, PartialEq)]
pub struct MutualFundNavProjection {
    /// The NAV disclosure date.
    pub nav_date: NaiveDate,
    /// AMFI scheme code.
    pub scheme_code: String,
    /// Net asset value per unit (scale 4).
    pub nav: Decimal,
    /// Repurchase price (scale 4), when reported.
    pub repurchase_price: Option<Decimal>,
    /// Sale price (scale 4), when reported.
    pub sale_price: Option<Decimal>,
}

/// One scheme's derived return metrics as of a date.
#[derive(Debug, Clone, PartialEq)]
pub struct MutualFundReturnProjection {
    /// The date the returns were computed.
    pub as_of_date: NaiveDate,
    /// AMFI scheme code.
    pub scheme_code: String,
    /// 1-year CAGR percent (scale 4), when derivable.
    pub cagr_1y: Option<Decimal>,
    /// Expense ratio percent (scale 4), when supplied.
    pub expense_ratio: Option<Decimal>,
}

/// One IPO issue's metadata as of a snapshot date.
#[derive(Debug, Clone, PartialEq)]
pub struct IpoProjection {
    /// The date the issue was observed.
    pub as_of_date: NaiveDate,
    /// Issue symbol.
    pub symbol: String,
    /// Company name.
    pub company_name: String,
    /// Listing board (`Main` / `SME`).
    pub board: String,
    /// Issue status (e.g. `Open`, `Listed`, `Closed`).
    pub status: String,
    /// Subscription open date.
    pub open_date: Option<NaiveDate>,
    /// Subscription close date.
    pub close_date: Option<NaiveDate>,
    /// Listing date.
    pub listing_date: Option<NaiveDate>,
    /// Lower end of the price band (scale 4), when reported.
    pub price_band_low: Option<Decimal>,
    /// Upper end of the price band (scale 4), when reported.
    pub price_band_high: Option<Decimal>,
    /// Fixed issue price (scale 4), when the issue is fixed-price.
    pub issue_price: Option<Decimal>,
    /// Minimum application lot size in shares (scale 4).
    pub lot_size: Option<Decimal>,
    /// Issue size in ₹ crore (scale 4).
    pub issue_size_cr: Option<Decimal>,
}

/// One IPO's subscription multiples as of a snapshot date.
#[derive(Debug, Clone, PartialEq)]
pub struct IpoSubscriptionProjection {
    /// The date the multiples were observed.
    pub as_of_date: NaiveDate,
    /// Issue symbol.
    pub symbol: String,
    /// QIB multiple (scale 4), when reported.
    pub qib: Option<Decimal>,
    /// NII multiple (scale 4), when reported.
    pub nii: Option<Decimal>,
    /// bNII (big NII) multiple (scale 4), when reported.
    pub bnii: Option<Decimal>,
    /// sNII (small NII) multiple (scale 4), when reported.
    pub snii: Option<Decimal>,
    /// RII multiple (scale 4), when reported.
    pub rii: Option<Decimal>,
    /// Total multiple (scale 4), when reported.
    pub total: Option<Decimal>,
}

/// Reader over the five fund/IPO projection directories.
#[derive(Clone)]
pub struct FundsIposProjectionReader {
    mutual_funds_dir: PathBuf,
    mf_nav_dir: PathBuf,
    mf_returns_dir: PathBuf,
    ipos_dir: PathBuf,
    ipo_subscriptions_dir: PathBuf,
}

impl FundsIposProjectionReader {
    /// Creates a reader rooted at `root`, the `tantra-data` catalog root.
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            mutual_funds_dir: root.join("catalog").join("mutual_funds"),
            mf_nav_dir: root.join("catalog").join("mf_nav"),
            mf_returns_dir: root.join("catalog").join("mf_returns"),
            ipos_dir: root.join("catalog").join("ipos"),
            ipo_subscriptions_dir: root.join("catalog").join("ipo_subscriptions"),
        }
    }

    /// Reads every mutual-fund scheme-metadata row.
    pub fn read_mutual_funds(
        &self,
    ) -> Result<Vec<MutualFundMetaProjection>, FundsIposProjectionError> {
        let mut rows = Vec::new();
        for file in parquet_files(&self.mutual_funds_dir)? {
            rows.extend(decode_file(&file, mutual_funds_of)?);
        }
        Ok(rows)
    }

    /// Reads every mutual-fund NAV observation.
    pub fn read_mf_nav(&self) -> Result<Vec<MutualFundNavProjection>, FundsIposProjectionError> {
        let mut rows = Vec::new();
        for file in parquet_files(&self.mf_nav_dir)? {
            rows.extend(decode_file(&file, mf_nav_of)?);
        }
        Ok(rows)
    }

    /// Reads every mutual-fund return metric.
    pub fn read_mf_returns(
        &self,
    ) -> Result<Vec<MutualFundReturnProjection>, FundsIposProjectionError> {
        let mut rows = Vec::new();
        for file in parquet_files(&self.mf_returns_dir)? {
            rows.extend(decode_file(&file, mf_returns_of)?);
        }
        Ok(rows)
    }

    /// Reads every IPO issue row.
    pub fn read_ipos(&self) -> Result<Vec<IpoProjection>, FundsIposProjectionError> {
        let mut rows = Vec::new();
        for file in parquet_files(&self.ipos_dir)? {
            rows.extend(decode_file(&file, ipos_of)?);
        }
        Ok(rows)
    }

    /// Reads every IPO subscription row.
    pub fn read_ipo_subscriptions(
        &self,
    ) -> Result<Vec<IpoSubscriptionProjection>, FundsIposProjectionError> {
        let mut rows = Vec::new();
        for file in parquet_files(&self.ipo_subscriptions_dir)? {
            rows.extend(decode_file(&file, ipo_subscriptions_of)?);
        }
        Ok(rows)
    }
}

/// Lists the sorted `*.parquet` files under `dir`, or an empty vector when absent.
fn parquet_files(dir: &Path) -> Result<Vec<PathBuf>, FundsIposProjectionError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "parquet"))
        .collect();
    files.sort();
    Ok(files)
}

/// Decodes every batch of one parquet file through `decode`.
fn decode_file<T>(
    file: &Path,
    decode: fn(&RecordBatch) -> Result<Vec<T>, FundsIposProjectionError>,
) -> Result<Vec<T>, FundsIposProjectionError> {
    let input = File::open(file)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(input)?;
    let mut rows = Vec::new();
    for batch in builder.build()? {
        rows.extend(decode(&batch?)?);
    }
    Ok(rows)
}

/// Resolves a column index, failing with [`FundsIposProjectionError::MissingColumn`].
fn column_index(
    schema: &arrow::datatypes::Schema,
    name: &'static str,
) -> Result<usize, FundsIposProjectionError> {
    schema
        .index_of(name)
        .map_err(|_| FundsIposProjectionError::MissingColumn(name))
}

/// Reads a non-null date32 value as a [`NaiveDate`].
fn date_at(
    column: &Date32Array,
    row: usize,
    name: &'static str,
) -> Result<NaiveDate, FundsIposProjectionError> {
    date32_to_naive_date(column.value(row)).ok_or_else(|| {
        FundsIposProjectionError::Arrow(arrow::error::ArrowError::InvalidArgumentError(format!(
            "invalid {name} value {}",
            column.value(row)
        )))
    })
}

/// Reads a nullable date32 value as `Option<NaiveDate>`.
fn optional_date_at(column: &Date32Array, row: usize) -> Option<NaiveDate> {
    if column.is_null(row) {
        None
    } else {
        date32_to_naive_date(column.value(row))
    }
}

/// Reads a nullable decimal128 value as `Option<Decimal>`.
fn optional_decimal_at(
    column: &Decimal128Array,
    row: usize,
    name: &'static str,
) -> Result<Option<Decimal>, FundsIposProjectionError> {
    if column.is_null(row) {
        return Ok(None);
    }
    Decimal::try_from_i128_with_scale(column.value(row), DECIMAL_SCALE)
        .map(Some)
        .map_err(|_| FundsIposProjectionError::BadDecimal { column: name })
}

/// Reads a nullable string column value as `Option<String>`.
fn optional_string_at(column: &StringArray, row: usize) -> Option<String> {
    if column.is_null(row) {
        None
    } else {
        Some(column.value(row).to_string())
    }
}

fn mutual_funds_of(
    batch: &RecordBatch,
) -> Result<Vec<MutualFundMetaProjection>, FundsIposProjectionError> {
    let schema = batch.schema();
    let i_as_of_date = column_index(&schema, "as_of_date")?;
    let i_scheme_code = column_index(&schema, "scheme_code")?;
    let i_scheme_name = column_index(&schema, "scheme_name")?;
    let i_amc = column_index(&schema, "amc")?;
    let i_category = column_index(&schema, "category")?;
    let i_isin_growth = column_index(&schema, "isin_growth")?;
    let i_isin_div_reinvestment = column_index(&schema, "isin_div_reinvestment")?;
    let i_plan = column_index(&schema, "plan")?;
    let i_option = column_index(&schema, "option")?;

    let as_of_dates = date32_array(batch.column(i_as_of_date), "as_of_date")?;
    let scheme_codes = string_array(batch.column(i_scheme_code), "scheme_code")?;
    let scheme_names = string_array(batch.column(i_scheme_name), "scheme_name")?;
    let amcs = string_array(batch.column(i_amc), "amc")?;
    let categories = string_array(batch.column(i_category), "category")?;
    let isin_growths = string_array(batch.column(i_isin_growth), "isin_growth")?;
    let isin_divs = string_array(
        batch.column(i_isin_div_reinvestment),
        "isin_div_reinvestment",
    )?;
    let plans = string_array(batch.column(i_plan), "plan")?;
    let options = string_array(batch.column(i_option), "option")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        rows.push(MutualFundMetaProjection {
            as_of_date: date_at(as_of_dates, row, "as_of_date")?,
            scheme_code: scheme_codes.value(row).to_string(),
            scheme_name: scheme_names.value(row).to_string(),
            amc: amcs.value(row).to_string(),
            category: categories.value(row).to_string(),
            isin_growth: optional_string_at(isin_growths, row),
            isin_div_reinvestment: optional_string_at(isin_divs, row),
            plan: plans.value(row).to_string(),
            option: options.value(row).to_string(),
        });
    }
    Ok(rows)
}

fn mf_nav_of(
    batch: &RecordBatch,
) -> Result<Vec<MutualFundNavProjection>, FundsIposProjectionError> {
    let schema = batch.schema();
    let i_nav_date = column_index(&schema, "nav_date")?;
    let i_scheme_code = column_index(&schema, "scheme_code")?;
    let i_nav = column_index(&schema, "nav")?;
    let i_repurchase = column_index(&schema, "repurchase_price")?;
    let i_sale = column_index(&schema, "sale_price")?;

    let nav_dates = date32_array(batch.column(i_nav_date), "nav_date")?;
    let scheme_codes = string_array(batch.column(i_scheme_code), "scheme_code")?;
    let navs = decimal128_array(batch.column(i_nav), "nav")?;
    let repurchases = decimal128_array(batch.column(i_repurchase), "repurchase_price")?;
    let sales = decimal128_array(batch.column(i_sale), "sale_price")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        rows.push(MutualFundNavProjection {
            nav_date: date_at(nav_dates, row, "nav_date")?,
            scheme_code: scheme_codes.value(row).to_string(),
            nav: optional_decimal_at(navs, row, "nav")?
                .ok_or_else(|| FundsIposProjectionError::BadDecimal { column: "nav" })?,
            repurchase_price: optional_decimal_at(repurchases, row, "repurchase_price")?,
            sale_price: optional_decimal_at(sales, row, "sale_price")?,
        });
    }
    Ok(rows)
}

fn mf_returns_of(
    batch: &RecordBatch,
) -> Result<Vec<MutualFundReturnProjection>, FundsIposProjectionError> {
    let schema = batch.schema();
    let i_as_of_date = column_index(&schema, "as_of_date")?;
    let i_scheme_code = column_index(&schema, "scheme_code")?;
    let i_cagr_1y = column_index(&schema, "cagr_1y")?;
    let i_expense_ratio = column_index(&schema, "expense_ratio")?;

    let as_of_dates = date32_array(batch.column(i_as_of_date), "as_of_date")?;
    let scheme_codes = string_array(batch.column(i_scheme_code), "scheme_code")?;
    let cagrs = decimal128_array(batch.column(i_cagr_1y), "cagr_1y")?;
    let expenses = decimal128_array(batch.column(i_expense_ratio), "expense_ratio")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        rows.push(MutualFundReturnProjection {
            as_of_date: date_at(as_of_dates, row, "as_of_date")?,
            scheme_code: scheme_codes.value(row).to_string(),
            cagr_1y: optional_decimal_at(cagrs, row, "cagr_1y")?,
            expense_ratio: optional_decimal_at(expenses, row, "expense_ratio")?,
        });
    }
    Ok(rows)
}

fn ipos_of(batch: &RecordBatch) -> Result<Vec<IpoProjection>, FundsIposProjectionError> {
    let schema = batch.schema();
    let i_as_of_date = column_index(&schema, "as_of_date")?;
    let i_symbol = column_index(&schema, "symbol")?;
    let i_company = column_index(&schema, "company_name")?;
    let i_board = column_index(&schema, "board")?;
    let i_status = column_index(&schema, "status")?;
    let i_open = column_index(&schema, "open_date")?;
    let i_close = column_index(&schema, "close_date")?;
    let i_listing = column_index(&schema, "listing_date")?;
    let i_low = column_index(&schema, "price_band_low")?;
    let i_high = column_index(&schema, "price_band_high")?;
    let i_issue_price = column_index(&schema, "issue_price")?;
    let i_lot = column_index(&schema, "lot_size")?;
    let i_size = column_index(&schema, "issue_size_cr")?;

    let as_of_dates = date32_array(batch.column(i_as_of_date), "as_of_date")?;
    let symbols = string_array(batch.column(i_symbol), "symbol")?;
    let companies = string_array(batch.column(i_company), "company_name")?;
    let boards = string_array(batch.column(i_board), "board")?;
    let statuses = string_array(batch.column(i_status), "status")?;
    let opens = date32_array(batch.column(i_open), "open_date")?;
    let closes = date32_array(batch.column(i_close), "close_date")?;
    let listings = date32_array(batch.column(i_listing), "listing_date")?;
    let lows = decimal128_array(batch.column(i_low), "price_band_low")?;
    let highs = decimal128_array(batch.column(i_high), "price_band_high")?;
    let issue_prices = decimal128_array(batch.column(i_issue_price), "issue_price")?;
    let lots = decimal128_array(batch.column(i_lot), "lot_size")?;
    let sizes = decimal128_array(batch.column(i_size), "issue_size_cr")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        rows.push(IpoProjection {
            as_of_date: date_at(as_of_dates, row, "as_of_date")?,
            symbol: symbols.value(row).to_string(),
            company_name: companies.value(row).to_string(),
            board: boards.value(row).to_string(),
            status: statuses.value(row).to_string(),
            open_date: optional_date_at(opens, row),
            close_date: optional_date_at(closes, row),
            listing_date: optional_date_at(listings, row),
            price_band_low: optional_decimal_at(lows, row, "price_band_low")?,
            price_band_high: optional_decimal_at(highs, row, "price_band_high")?,
            issue_price: optional_decimal_at(issue_prices, row, "issue_price")?,
            lot_size: optional_decimal_at(lots, row, "lot_size")?,
            issue_size_cr: optional_decimal_at(sizes, row, "issue_size_cr")?,
        });
    }
    Ok(rows)
}

fn ipo_subscriptions_of(
    batch: &RecordBatch,
) -> Result<Vec<IpoSubscriptionProjection>, FundsIposProjectionError> {
    let schema = batch.schema();
    let i_as_of_date = column_index(&schema, "as_of_date")?;
    let i_symbol = column_index(&schema, "symbol")?;
    let i_qib = column_index(&schema, "qib")?;
    let i_nii = column_index(&schema, "nii")?;
    let i_bnii = column_index(&schema, "bnii")?;
    let i_snii = column_index(&schema, "snii")?;
    let i_rii = column_index(&schema, "rii")?;
    let i_total = column_index(&schema, "total")?;

    let as_of_dates = date32_array(batch.column(i_as_of_date), "as_of_date")?;
    let symbols = string_array(batch.column(i_symbol), "symbol")?;
    let qibs = decimal128_array(batch.column(i_qib), "qib")?;
    let niis = decimal128_array(batch.column(i_nii), "nii")?;
    let bniis = decimal128_array(batch.column(i_bnii), "bnii")?;
    let sniis = decimal128_array(batch.column(i_snii), "snii")?;
    let riis = decimal128_array(batch.column(i_rii), "rii")?;
    let totals = decimal128_array(batch.column(i_total), "total")?;

    let mut rows = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        rows.push(IpoSubscriptionProjection {
            as_of_date: date_at(as_of_dates, row, "as_of_date")?,
            symbol: symbols.value(row).to_string(),
            qib: optional_decimal_at(qibs, row, "qib")?,
            nii: optional_decimal_at(niis, row, "nii")?,
            bnii: optional_decimal_at(bniis, row, "bnii")?,
            snii: optional_decimal_at(sniis, row, "snii")?,
            rii: optional_decimal_at(riis, row, "rii")?,
            total: optional_decimal_at(totals, row, "total")?,
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
) -> Result<&'a Date32Array, FundsIposProjectionError> {
    column
        .as_any()
        .downcast_ref::<Date32Array>()
        .ok_or(FundsIposProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`StringArray`], failing on a wrong type.
fn string_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a StringArray, FundsIposProjectionError> {
    column
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or(FundsIposProjectionError::MissingColumn(name))
}

/// Downcasts a column to a [`Decimal128Array`], failing on a wrong type.
fn decimal128_array<'a>(
    column: &'a dyn Array,
    name: &'static str,
) -> Result<&'a Decimal128Array, FundsIposProjectionError> {
    column
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .ok_or(FundsIposProjectionError::MissingColumn(name))
}
