//! Integration tests for the `tantra-data` projections readers
//! (gaps `tantra-data` 0007 and 0008).

use std::sync::Arc;

use arrow::array::{Date32Array, Decimal128Array, Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::arrow_writer::ArrowWriter;
use rust_decimal::Decimal;
use tantra_market_store::import::corporate_actions_projection::CorporateActionsProjectionReader;
use tantra_market_store::import::filings_projection::FilingsProjectionReader;
use tempfile::tempdir;

#[test]
fn filings_reader_decodes_rows_and_preserves_nullable_source_url() {
    let dir = tempdir().unwrap();
    let filings_dir = dir.path().join("catalog").join("filings");
    std::fs::create_dir_all(&filings_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("filing_date", DataType::Date32, false),
        Field::new("instrument", DataType::Utf8, false),
        Field::new("filing_type", DataType::Utf8, false),
        Field::new("title", DataType::Utf8, false),
        Field::new("source_url", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![0i32, 1])),
            Arc::new(StringArray::from(vec!["NSE:RELIANCE-EQ", "NSE:TCS-EQ"])),
            Arc::new(StringArray::from(vec!["corporate_action", "insider_trade"])),
            Arc::new(StringArray::from(vec![
                "Board meeting outcome",
                "Promoter disclosure",
            ])),
            Arc::new(StringArray::from(vec![
                Some("https://example.com/filing/1"),
                None,
            ])),
        ],
    )
    .unwrap();

    let path = filings_dir.join("2024-01-05.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = FilingsProjectionReader::new(dir.path()).read().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].filing_date,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
    );
    assert_eq!(rows[0].instrument, "NSE:RELIANCE-EQ");
    assert_eq!(rows[0].filing_type, "corporate_action");
    assert_eq!(rows[0].title, "Board meeting outcome");
    assert_eq!(
        rows[0].source_url.as_deref(),
        Some("https://example.com/filing/1")
    );
    assert_eq!(
        rows[1].filing_date,
        NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()
    );
    assert_eq!(rows[1].instrument, "NSE:TCS-EQ");
    assert_eq!(rows[1].filing_type, "insider_trade");
    assert_eq!(rows[1].title, "Promoter disclosure");
    assert_eq!(rows[1].source_url, None);
}

#[test]
fn corporate_actions_reader_decodes_decimal_and_nullable_columns() {
    let dir = tempdir().unwrap();
    let actions_dir = dir.path().join("catalog").join("corporate_actions");
    std::fs::create_dir_all(&actions_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("ex_date", DataType::Date32, false),
        Field::new("instrument", DataType::Utf8, false),
        Field::new("action_type", DataType::Utf8, false),
        Field::new("old_shares", DataType::Int32, true),
        Field::new("new_shares", DataType::Int32, true),
        Field::new("per_share", DataType::Decimal128(18, 4), true),
        Field::new("issue_price", DataType::Decimal128(18, 4), true),
        Field::new("currency", DataType::Utf8, false),
    ]));
    let per_share = Decimal128Array::from(vec![Some(12_500i128), None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let issue_price = Decimal128Array::from(vec![Some(100_000i128), None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![0i32, 7])),
            Arc::new(StringArray::from(vec![
                "NSE:RELIANCE-EQ",
                "NSE:HDFCBANK-EQ",
            ])),
            Arc::new(StringArray::from(vec!["split", "bonus"])),
            Arc::new(Int32Array::from(vec![Some(10), None])),
            Arc::new(Int32Array::from(vec![Some(50), None])),
            Arc::new(per_share),
            Arc::new(issue_price),
            Arc::new(StringArray::from(vec!["INR", "INR"])),
        ],
    )
    .unwrap();

    let path = actions_dir.join("2024-06-14.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = CorporateActionsProjectionReader::new(dir.path())
        .read()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].ex_date,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
    );
    assert_eq!(rows[0].instrument, "NSE:RELIANCE-EQ");
    assert_eq!(rows[0].action_type, "split");
    assert_eq!(rows[0].old_shares, Some(10));
    assert_eq!(rows[0].new_shares, Some(50));
    assert_eq!(
        rows[0].per_share,
        Some(Decimal::try_from_i128_with_scale(12_500, 4).unwrap())
    );
    assert_eq!(
        rows[0].issue_price,
        Some(Decimal::try_from_i128_with_scale(100_000, 4).unwrap())
    );
    assert_eq!(rows[0].currency, "INR");
    assert_eq!(
        rows[1].ex_date,
        NaiveDate::from_ymd_opt(1970, 1, 8).unwrap()
    );
    assert_eq!(rows[1].instrument, "NSE:HDFCBANK-EQ");
    assert_eq!(rows[1].action_type, "bonus");
    assert_eq!(rows[1].old_shares, None);
    assert_eq!(rows[1].new_shares, None);
    assert_eq!(rows[1].per_share, None);
    assert_eq!(rows[1].issue_price, None);
    assert_eq!(rows[1].currency, "INR");
}

#[test]
fn readers_return_empty_for_a_missing_catalog_directory() {
    let dir = tempdir().unwrap();
    assert!(
        FilingsProjectionReader::new(dir.path())
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        CorporateActionsProjectionReader::new(dir.path())
            .read()
            .unwrap()
            .is_empty()
    );
}
