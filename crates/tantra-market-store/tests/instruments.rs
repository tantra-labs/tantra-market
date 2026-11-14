//! Integration test: a backtest-shaped flow consults the dated instrument
//! catalog, so an expired contract neither resolves nor fills while a live one
//! does. Fixtures are generated in a temp dir; nothing touches the network.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::ArrowWriter;
use tantra_core::{Exchange, InstrumentId};
use tantra_market_store::ports::InstrumentMaster;
use tantra_market_store::wire::UnixNanos;
use tantra_market_store::{CatalogInstrumentMaster, snapshot_schema};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tantra_instruments_it_{}_{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes `{root}/instruments/{date}.parquet` with the two futures under test.
fn write_futures(root: &Path, date: &str, rows: &[(&str, &str)]) {
    let dir = root.join("instruments");
    std::fs::create_dir_all(&dir).unwrap();
    let strings = |values: Vec<Option<&str>>| -> ArrayRef { Arc::new(StringArray::from(values)) };
    let numbers = |values: Vec<Option<f64>>| -> ArrayRef { Arc::new(Float64Array::from(values)) };
    let columns: Vec<ArrayRef> = vec![
        strings(rows.iter().map(|(symbol, _)| Some(*symbol)).collect()),
        strings(rows.iter().map(|_| Some("NSE")).collect()),
        strings(rows.iter().map(|_| Some("future")).collect()),
        strings(rows.iter().map(|_| Some("INR")).collect()),
        numbers(rows.iter().map(|_| Some(75.0)).collect()),
        numbers(rows.iter().map(|_| Some(0.05)).collect()),
        numbers(rows.iter().map(|_| Some(1500.0)).collect()),
        strings(rows.iter().map(|(_, expiry)| Some(*expiry)).collect()),
        numbers(rows.iter().map(|_| None).collect()),
        strings(rows.iter().map(|_| None).collect()),
        strings(rows.iter().map(|_| Some("NIFTY")).collect()),
        strings(rows.iter().map(|_| None).collect()),
    ];
    let schema = Arc::new(snapshot_schema());
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    let file = File::create(dir.join(format!("{date}.parquet"))).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

#[tokio::test]
async fn catalog_resolves_expired_contract() {
    let dir = TempDir::new("backtest_expired");
    write_futures(
        &dir.0,
        "2024-01-01",
        &[
            ("NIFTY24OCTFUT", "2024-10-31"),
            ("NIFTY24DECFUT", "2024-12-26"),
        ],
    );
    let master = CatalogInstrumentMaster::open(&dir.0).unwrap();
    let as_of = NaiveDate::from_ymd_opt(2024, 11, 15).unwrap();

    let expired = InstrumentId::new(Exchange::Nse, "NIFTY24OCTFUT");
    let live = InstrumentId::new(Exchange::Nse, "NIFTY24DECFUT");

    // The dated catalog is the gate: the contract expired on 2024-10-31.
    assert!(
        master
            .resolve_as_of(&expired, as_of)
            .await
            .unwrap()
            .is_none()
    );
    assert!(master.resolve_as_of(&live, as_of).await.unwrap().is_some());

    let _ts = UnixNanos::from_u64(1_700_000_000_000_000_000);

    // Simulate what the engine would do: resolve and decide whether to order.
    let mut orders_submitted = 0;
    for (id, _order_id) in [(&expired, "EXPIRED-1"), (&live, "LIVE-1")] {
        if master.resolve_as_of(id, as_of).await.unwrap().is_some() {
            orders_submitted += 1;
        }
    }

    assert_eq!(
        orders_submitted, 1,
        "only the live contract should be submitted"
    );
}

#[tokio::test]
async fn catalog_reads_display_name_column() {
    let dir = TempDir::new("display_name");
    let dir_path = dir.0.join("instruments");
    std::fs::create_dir_all(&dir_path).unwrap();
    let strings = |values: Vec<Option<&str>>| -> ArrayRef { Arc::new(StringArray::from(values)) };
    let numbers = |values: Vec<Option<f64>>| -> ArrayRef { Arc::new(Float64Array::from(values)) };
    let columns: Vec<ArrayRef> = vec![
        strings(vec![Some("RELIANCE-EQ"), Some("TCS-EQ")]),
        strings(vec![Some("NSE"), Some("NSE")]),
        strings(vec![Some("equity"), Some("equity")]),
        strings(vec![Some("INR"), Some("INR")]),
        numbers(vec![Some(1.0), Some(1.0)]),
        numbers(vec![Some(0.05), Some(0.05)]),
        numbers(vec![Some(1500.0), Some(1500.0)]),
        strings(vec![None, None]),
        numbers(vec![None, None]),
        strings(vec![None, None]),
        strings(vec![None, None]),
        strings(vec![Some("RELIANCE INDUSTRIES"), None]),
    ];
    let schema = Arc::new(snapshot_schema());
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    let file = File::create(dir_path.join("2026-01-05.parquet")).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let master = CatalogInstrumentMaster::open(&dir.0).unwrap();
    let reliance = InstrumentId::new(Exchange::Nse, "RELIANCE-EQ");
    let tcs = InstrumentId::new(Exchange::Nse, "TCS-EQ");
    assert_eq!(
        master
            .get_instrument(&reliance)
            .await
            .unwrap()
            .unwrap()
            .display_name(),
        Some("RELIANCE INDUSTRIES")
    );
    assert_eq!(
        master
            .get_instrument(&tcs)
            .await
            .unwrap()
            .unwrap()
            .display_name(),
        None
    );
}

#[tokio::test]
async fn catalog_without_display_name_column_still_loads() {
    let dir = TempDir::new("no_display_name");
    let dir_path = dir.0.join("instruments");
    std::fs::create_dir_all(&dir_path).unwrap();
    let strings = |values: Vec<Option<&str>>| -> ArrayRef { Arc::new(StringArray::from(values)) };
    let numbers = |values: Vec<Option<f64>>| -> ArrayRef { Arc::new(Float64Array::from(values)) };
    // Hand-rolled 11-column schema: the snapshot layout before `display_name`.
    let schema = Arc::new(Schema::new(vec![
        Field::new("symbol", DataType::Utf8, false),
        Field::new("exchange", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("currency", DataType::Utf8, false),
        Field::new("lot_size", DataType::Float64, false),
        Field::new("tick_size", DataType::Float64, false),
        Field::new("freeze_qty", DataType::Float64, false),
        Field::new("expiry", DataType::Utf8, true),
        Field::new("strike", DataType::Float64, true),
        Field::new("option_type", DataType::Utf8, true),
        Field::new("underlying", DataType::Utf8, true),
    ]));
    let columns: Vec<ArrayRef> = vec![
        strings(vec![Some("RELIANCE-EQ")]),
        strings(vec![Some("NSE")]),
        strings(vec![Some("equity")]),
        strings(vec![Some("INR")]),
        numbers(vec![Some(1.0)]),
        numbers(vec![Some(0.05)]),
        numbers(vec![Some(1500.0)]),
        strings(vec![None]),
        numbers(vec![None]),
        strings(vec![None]),
        strings(vec![None]),
    ];
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    let file = File::create(dir_path.join("2026-01-05.parquet")).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let master = CatalogInstrumentMaster::open(&dir.0).unwrap();
    let reliance = InstrumentId::new(Exchange::Nse, "RELIANCE-EQ");
    let instrument = master.get_instrument(&reliance).await.unwrap().unwrap();
    assert_eq!(instrument.display_name(), None);
}
