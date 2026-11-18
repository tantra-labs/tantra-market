//! A dated instrument catalog.
//!
//! [`CatalogInstrumentMaster`] reads annotated instrument snapshots — either
//! Lakehouse v3 `reference/instruments` CAS / Hive view manifests or legacy
//! `{root}/instruments/YYYY-MM-DD.parquet` — and answers the
//! [`InstrumentMaster`] port as of a backtest date.
//!
//! # Snapshot columns (`Zero-Floats`)
//!
//! `symbol`, `exchange`, `kind`, `currency`, `expiry`, `option_type`,
//! `underlying` and `display_name` are UTF-8 (`Utf8` or `LargeUtf8`);
//! `lot_size`, `tick_size`, `freeze_qty` and nullable `strike` use
//! `Decimal128(18, 4)` in [`snapshot_schema`]. Legacy `Float64` columns are
//! rejected by [`CatalogInstrumentMaster::open`] and accepted only via
//! [`CatalogInstrumentMaster::open_legacy_float64`] during migration tests.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, ArrayRef, Date32Array, Decimal128Array, Float64Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use chrono::NaiveDate;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tantra_core::{Currency, Exchange, InstrumentId, InstrumentKind, OptionType};
use thiserror::Error;

use crate::lakehouse::{PRICE_PRECISION, PRICE_SCALE};
use crate::manifest::{
    ManifestError, ManifestResolver, arrow_utf8_array, collect_parquet_files_recursive,
};
use crate::ports::{DatedInstrument, InstrumentMaster, PortError, PortResult};
use crate::wire::Instrument;

/// The directory under the legacy catalog root that holds dated snapshots.
pub const SNAPSHOT_DIR: &str = "instruments";

/// Days from 0001-01-01 to 1970-01-01 for Arrow `Date32` conversion.
const UNIX_EPOCH_DAYS_FROM_CE: i64 = 719_163;

/// The canonical Zero-Floats `Decimal128(18, 4)` column layout of a dated snapshot file.
pub fn snapshot_schema() -> Schema {
    let dec = DataType::Decimal128(PRICE_PRECISION, PRICE_SCALE);
    Schema::new(vec![
        Field::new("symbol", DataType::Utf8, false),
        Field::new("exchange", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("currency", DataType::Utf8, false),
        Field::new("lot_size", dec.clone(), false),
        Field::new("tick_size", dec.clone(), false),
        Field::new("freeze_qty", dec.clone(), false),
        Field::new("expiry", DataType::Utf8, true),
        Field::new("strike", dec, true),
        Field::new("option_type", DataType::Utf8, true),
        Field::new("underlying", DataType::Utf8, true),
        Field::new("display_name", DataType::Utf8, true),
    ])
}

/// Why a [`CatalogInstrumentMaster`] could not be opened or read.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// The snapshot directory or one of its files could not be read.
    #[error("cannot read instrument catalog: {0}")]
    Io(#[from] std::io::Error),
    /// A `.parquet` file could not be decoded.
    #[error("parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// An Arrow array could not be converted.
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// A Lakehouse v3 manifest error occurred.
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    /// A snapshot file is not named `YYYY-MM-DD.parquet`.
    #[error("snapshot file name {0:?} is not YYYY-MM-DD.parquet")]
    FileName(String),
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
    /// A value could not be parsed as a `YYYY-MM-DD` date.
    #[error("invalid date {value:?} in column {column}")]
    BadDate {
        /// Name of the column.
        column: &'static str,
        /// The unparsable value.
        value: String,
    },
    /// A value is not a known member of its enum.
    #[error("unknown {column} {value:?}")]
    UnknownValue {
        /// Name of the column.
        column: &'static str,
        /// The unknown value.
        value: String,
    },
}

impl From<CatalogError> for PortError {
    fn from(error: CatalogError) -> Self {
        PortError::Internal(error.to_string())
    }
}

/// An [`InstrumentMaster`] over dated Parquet snapshots (Lakehouse v3 or legacy directory).
#[derive(Clone, Debug)]
pub struct CatalogInstrumentMaster {
    snapshots: BTreeMap<NaiveDate, Vec<DatedInstrument>>,
}

impl CatalogInstrumentMaster {
    /// Loads instrument snapshots from `root`, resolving v3 `reference/instruments`
    /// manifests/views first and falling back to `{root}/instruments/YYYY-MM-DD.parquet`.
    ///
    /// Enforces the Zero-Floats invariant: numeric columns (`lot_size`, `tick_size`,
    /// `freeze_qty`, `strike`) must be `Decimal128` (or integer), rejecting `Float64`.
    pub fn open(root: &Path) -> Result<Self, CatalogError> {
        Self::open_with_options(root, None, true, false)
    }

    /// Opens at a specific v3 ref (`refs/tags/<tag>` or commit SHA), resolving
    /// `reference/instruments` from that manifest snapshot.
    pub fn open_at_ref(
        root: &Path,
        reference: Option<&str>,
        legacy_fallback: bool,
    ) -> Result<Self, CatalogError> {
        Self::open_with_options(root, reference, legacy_fallback, false)
    }

    /// Migration-only loader accepting legacy `Float64` snapshot columns.
    pub fn open_legacy_float64(root: &Path) -> Result<Self, CatalogError> {
        Self::open_with_options(root, None, true, true)
    }

    /// Configurable loader over v3 manifests (`reference/instruments`) and legacy `instruments/`.
    pub fn open_with_options(
        root: &Path,
        reference: Option<&str>,
        legacy_fallback: bool,
        allow_legacy_float64: bool,
    ) -> Result<Self, CatalogError> {
        let resolver = ManifestResolver::new(root).with_legacy_fallback(legacy_fallback);
        let mut dated_files: Vec<(NaiveDate, PathBuf)> = Vec::new();

        if resolver.has_v3_layout() {
            let (maybe_commit, pairs) = resolver.resolve_partition_entries(
                "reference",
                "instruments",
                None,
                None,
                None,
                None,
                reference,
            )?;
            for (entry, path) in pairs {
                let d = maybe_commit
                    .as_ref()
                    .map(|c| c.as_of_ist)
                    .unwrap_or_else(|| entry.max_ts.date_naive());
                dated_files.push((d, path));
            }
        }

        if dated_files.is_empty() {
            let direct_v3 = root.join("reference").join("instruments");
            if direct_v3.is_dir() {
                let mut paths = Vec::new();
                collect_parquet_files_recursive(&direct_v3, &mut paths)?;
                paths.sort();
                for path in paths {
                    let d = snapshot_date_or_default(&path);
                    dated_files.push((d, path));
                }
            }
        }

        if dated_files.is_empty() && legacy_fallback {
            let dir = root.join(SNAPSHOT_DIR);
            let mut paths = Vec::new();
            for entry in std::fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.extension().is_some_and(|ext| ext == "parquet") {
                    paths.push(path);
                }
            }
            paths.sort();
            for path in paths {
                let d = snapshot_date(&path)?;
                dated_files.push((d, path));
            }
        }

        let mut snapshots = BTreeMap::new();
        for (date, path) in dated_files {
            read_snapshot_into(&path, date, allow_legacy_float64, &mut snapshots)?;
        }
        Ok(Self { snapshots })
    }

    /// The latest snapshot of `id` at or before `as_of`.
    fn latest_as_of(&self, id: &InstrumentId, as_of: NaiveDate) -> Option<&DatedInstrument> {
        self.snapshots
            .range(..=as_of)
            .rev()
            .flat_map(|(_, records)| records.iter())
            .find(|dated| dated.instrument.id() == id)
    }

    /// The latest snapshot of `id` on any date.
    fn latest(&self, id: &InstrumentId) -> Option<&DatedInstrument> {
        self.snapshots
            .values()
            .rev()
            .flat_map(|records| records.iter())
            .find(|dated| dated.instrument.id() == id)
    }

    /// One row per instrument: the newest snapshot at or before each date.
    fn merged(&self) -> BTreeMap<&InstrumentId, &DatedInstrument> {
        let mut merged: BTreeMap<&InstrumentId, &DatedInstrument> = BTreeMap::new();
        for records in self.snapshots.values() {
            for dated in records {
                merged.insert(dated.instrument.id(), dated);
            }
        }
        merged
    }

    /// Every in-force, non-expired record on `as_of`, in instrument-id order.
    fn in_force(&self, as_of: NaiveDate) -> Vec<DatedInstrument> {
        let mut merged: BTreeMap<&InstrumentId, &DatedInstrument> = BTreeMap::new();
        for (_, records) in self.snapshots.range(..=as_of) {
            for dated in records {
                merged.insert(dated.instrument.id(), dated);
            }
        }
        merged
            .into_values()
            .filter(|dated| !expired(dated, as_of))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl InstrumentMaster for CatalogInstrumentMaster {
    async fn get_instrument(&self, id: &InstrumentId) -> PortResult<Option<Instrument>> {
        Ok(self.latest(id).map(|dated| dated.instrument.clone()))
    }

    async fn list_instruments(&self) -> PortResult<Vec<Instrument>> {
        Ok(self
            .merged()
            .into_values()
            .map(|dated| dated.instrument.clone())
            .collect())
    }

    async fn list_chain(
        &self,
        underlying: &InstrumentId,
        as_of: Option<NaiveDate>,
    ) -> PortResult<Vec<Instrument>> {
        let records = match as_of {
            Some(date) => self.in_force(date),
            None => self
                .merged()
                .into_values()
                .cloned()
                .collect::<Vec<DatedInstrument>>(),
        };
        let mut chain: Vec<Instrument> = records
            .into_iter()
            .map(|dated| dated.instrument)
            .filter(|inst| {
                inst.kind() == tantra_core::InstrumentKind::Option
                    && inst
                        .underlying()
                        .is_some_and(|u| u.eq_ignore_ascii_case(&underlying.symbol))
            })
            .collect();
        chain.sort_by(|a, b| {
            let expiry = |i: &Instrument| i.expiry().unwrap_or(NaiveDate::MAX);
            let strike = |i: &Instrument| i.strike().unwrap_or(f64::INFINITY);
            let ot = |i: &Instrument| match i.option_type() {
                Some(tantra_core::OptionType::Call) => 0u8,
                Some(tantra_core::OptionType::Put) => 1u8,
                None => 2u8,
            };
            expiry(a)
                .cmp(&expiry(b))
                .then_with(|| {
                    strike(a)
                        .partial_cmp(&strike(b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| ot(a).cmp(&ot(b)))
        });
        Ok(chain)
    }

    async fn snapshot(&self, as_of: NaiveDate) -> PortResult<Vec<DatedInstrument>> {
        Ok(self.in_force(as_of))
    }

    async fn resolve_as_of(
        &self,
        id: &InstrumentId,
        as_of: NaiveDate,
    ) -> PortResult<Option<Instrument>> {
        Ok(self.latest_as_of(id, as_of).and_then(|dated| {
            if expired(dated, as_of) {
                None
            } else {
                Some(dated.instrument.clone())
            }
        }))
    }
}

/// True when the record's contract expired strictly before `as_of`.
fn expired(dated: &DatedInstrument, as_of: NaiveDate) -> bool {
    dated
        .instrument
        .expiry()
        .is_some_and(|expiry| expiry < as_of)
}

/// Parses the `YYYY-MM-DD` stem of a snapshot path.
fn snapshot_date(path: &Path) -> Result<NaiveDate, CatalogError> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    NaiveDate::parse_from_str(stem, "%Y-%m-%d").map_err(|_| CatalogError::FileName(file_name(path)))
}

fn snapshot_date_or_default(path: &Path) -> NaiveDate {
    snapshot_date(path).unwrap_or_else(|_| NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch"))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn read_snapshot_into(
    path: &Path,
    default_date: NaiveDate,
    allow_legacy_float64: bool,
    snapshots: &mut BTreeMap<NaiveDate, Vec<DatedInstrument>>,
) -> Result<(), CatalogError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    for batch in reader {
        append_batch(&batch?, default_date, allow_legacy_float64, snapshots)?;
    }
    Ok(())
}

fn append_batch(
    batch: &RecordBatch,
    default_date: NaiveDate,
    allow_legacy_float64: bool,
    snapshots: &mut BTreeMap<NaiveDate, Vec<DatedInstrument>>,
) -> Result<(), CatalogError> {
    let schema = batch.schema();
    let column = |name: &'static str| -> Result<ArrayRef, CatalogError> {
        let index = schema
            .index_of(name)
            .map_err(|_| CatalogError::MissingColumn(name))?;
        Ok(batch.column(index).clone())
    };
    let utf8 = |name: &'static str| -> Result<StringArray, CatalogError> {
        let col = column(name)?;
        arrow_utf8_array(col.as_ref()).map_err(|_| CatalogError::WrongType {
            name,
            got: col.data_type().clone(),
        })
    };
    let number = |name: &'static str| -> Result<Vec<Option<f64>>, CatalogError> {
        read_numeric_column(column(name)?.as_ref(), name, allow_legacy_float64)
    };

    let symbols = utf8("symbol")?;
    let exchanges = utf8("exchange")?;
    let kinds = utf8("kind")?;
    let currencies = utf8("currency")?;
    let lot_sizes = number("lot_size")?;
    let tick_sizes = number("tick_size")?;
    let freeze_qtys = number("freeze_qty")?;
    let expiries = utf8("expiry")?;
    let strikes = number("strike")?;
    let option_types = utf8("option_type")?;
    let underlyings = utf8("underlying")?;
    let display_names = match schema.index_of("display_name") {
        Ok(index) => {
            let col = batch.column(index);
            Some(
                arrow_utf8_array(col.as_ref()).map_err(|_| CatalogError::WrongType {
                    name: "display_name",
                    got: col.data_type().clone(),
                })?,
            )
        }
        Err(_) => None,
    };
    let row_dates = match schema
        .index_of("effective_from")
        .or_else(|_| schema.index_of("as_of"))
    {
        Ok(index) => Some(read_date_column(
            batch.column(index).as_ref(),
            "effective_from",
        )?),
        Err(_) => None,
    };

    for row in 0..batch.num_rows() {
        let row_date = row_dates
            .as_ref()
            .and_then(|d| d[row])
            .unwrap_or(default_date);
        let expiry = optional(&expiries, row, |value| parse_date(value, "expiry"))?;
        let option_type = optional(&option_types, row, parse_option_type)?;
        let display_name = display_names
            .as_ref()
            .filter(|names| !names.is_null(row) && !names.value(row).is_empty())
            .map(|names| names.value(row).to_string());
        let lot_size = lot_sizes[row].ok_or(CatalogError::MissingColumn("lot_size"))?;
        let tick_size = tick_sizes[row].ok_or(CatalogError::MissingColumn("tick_size"))?;
        let freeze_qty = freeze_qtys[row].ok_or(CatalogError::MissingColumn("freeze_qty"))?;
        let instrument = Instrument::new(
            InstrumentId::new(parse_exchange(exchanges.value(row))?, symbols.value(row)),
            parse_kind(kinds.value(row))?,
            parse_currency(currencies.value(row))?,
            lot_size,
            tick_size,
        )
        .with_derivative(
            expiry,
            strikes[row],
            option_type,
            optional_string(&underlyings, row),
        );
        let instrument = match display_name {
            Some(name) => instrument.with_display_name(name),
            None => instrument,
        };
        snapshots
            .entry(row_date)
            .or_default()
            .push(DatedInstrument {
                instrument,
                freeze_qty,
                effective_from: row_date,
                effective_to: None,
            });
    }
    Ok(())
}

fn read_numeric_column(
    array: &dyn Array,
    name: &'static str,
    allow_legacy_float64: bool,
) -> Result<Vec<Option<f64>>, CatalogError> {
    let mut out = Vec::with_capacity(array.len());
    match array.data_type() {
        DataType::Decimal128(_, scale) => {
            let arr = array
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .ok_or_else(|| CatalogError::WrongType {
                    name,
                    got: array.data_type().clone(),
                })?;
            let scale_u32 = (*scale).max(0) as u32;
            for i in 0..arr.len() {
                if arr.is_null(i) {
                    out.push(None);
                } else {
                    let dec = Decimal::from_i128_with_scale(arr.value(i), scale_u32);
                    out.push(Some(dec.to_f64().unwrap_or(0.0)));
                }
            }
        }
        dt if dt.is_integer() => {
            let casted = cast(array, &DataType::Decimal128(PRICE_PRECISION, PRICE_SCALE))?;
            let arr = casted
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .expect("integer casts to Decimal128");
            for i in 0..arr.len() {
                if arr.is_null(i) {
                    out.push(None);
                } else {
                    let dec = Decimal::from_i128_with_scale(arr.value(i), PRICE_SCALE as u32);
                    out.push(Some(dec.to_f64().unwrap_or(0.0)));
                }
            }
        }
        DataType::Float64 | DataType::Float32 if allow_legacy_float64 => {
            let casted = cast(array, &DataType::Float64)?;
            let arr = casted
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("cast to Float64");
            for i in 0..arr.len() {
                out.push((!arr.is_null(i)).then(|| arr.value(i)));
            }
        }
        got => {
            return Err(CatalogError::WrongType {
                name,
                got: got.clone(),
            });
        }
    }
    Ok(out)
}

fn read_date_column(
    array: &dyn Array,
    column: &'static str,
) -> Result<Vec<Option<NaiveDate>>, CatalogError> {
    let mut out = Vec::with_capacity(array.len());
    match array.data_type() {
        DataType::Date32 => {
            let arr = array
                .as_any()
                .downcast_ref::<Date32Array>()
                .ok_or_else(|| CatalogError::WrongType {
                    name: column,
                    got: array.data_type().clone(),
                })?;
            for i in 0..arr.len() {
                if arr.is_null(i) {
                    out.push(None);
                } else {
                    let days_ce = i64::from(arr.value(i)) + UNIX_EPOCH_DAYS_FROM_CE;
                    out.push(
                        i32::try_from(days_ce)
                            .ok()
                            .and_then(NaiveDate::from_num_days_from_ce_opt),
                    );
                }
            }
        }
        DataType::Utf8 | DataType::LargeUtf8 => {
            let arr = arrow_utf8_array(array)?;
            for i in 0..arr.len() {
                if arr.is_null(i) || arr.value(i).is_empty() {
                    out.push(None);
                } else {
                    out.push(Some(parse_date(arr.value(i), column)?));
                }
            }
        }
        got => {
            return Err(CatalogError::WrongType {
                name: column,
                got: got.clone(),
            });
        }
    }
    Ok(out)
}

fn optional_string(array: &StringArray, row: usize) -> Option<String> {
    (!array.is_null(row)).then(|| array.value(row).to_string())
}

fn optional<T>(
    array: &StringArray,
    row: usize,
    parse: impl Fn(&str) -> Result<T, CatalogError>,
) -> Result<Option<T>, CatalogError> {
    if array.is_null(row) {
        Ok(None)
    } else {
        parse(array.value(row)).map(Some)
    }
}

fn parse_date(value: &str, column: &'static str) -> Result<NaiveDate, CatalogError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| CatalogError::BadDate {
        column,
        value: value.to_string(),
    })
}

fn parse_kind(value: &str) -> Result<InstrumentKind, CatalogError> {
    from_json(value, "kind")
}

fn parse_currency(value: &str) -> Result<Currency, CatalogError> {
    Currency::from_code(value).map_err(|_| CatalogError::UnknownValue {
        column: "currency",
        value: value.to_string(),
    })
}

fn parse_option_type(value: &str) -> Result<OptionType, CatalogError> {
    match value.to_uppercase().as_str() {
        "CE" => Ok(OptionType::Call),
        "PE" => Ok(OptionType::Put),
        _ => from_json(value, "option_type"),
    }
}

fn parse_exchange(value: &str) -> Result<Exchange, CatalogError> {
    from_json(value, "exchange")
}

fn from_json<T: serde::de::DeserializeOwned>(
    value: &str,
    column: &'static str,
) -> Result<T, CatalogError> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).map_err(|_| {
        CatalogError::UnknownValue {
            column,
            value: value.to_string(),
        }
    })
}
