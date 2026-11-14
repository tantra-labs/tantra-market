//! Integration tests for the `tantra-data` projections readers
//! (gaps `tantra-data` 0007 and 0008).

use std::sync::Arc;

use arrow::array::{Date32Array, Decimal128Array, Int32Array, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::arrow_writer::ArrowWriter;
use rust_decimal::Decimal;
use tantra_market_store::import::block_deals_projection::BlockDealsProjectionReader;
use tantra_market_store::import::corporate_actions_projection::CorporateActionsProjectionReader;
use tantra_market_store::import::delivery_projection::DeliveryProjectionReader;
use tantra_market_store::import::economic_calendar_projection::EconomicCalendarProjectionReader;
use tantra_market_store::import::filings_projection::FilingsProjectionReader;
use tantra_market_store::import::fundamentals_projection::FundamentalsProjectionReader;
use tantra_market_store::import::funds_ipos_projection::FundsIposProjectionReader;
use tantra_market_store::import::institutional_flows_projection::InstitutionalFlowsProjectionReader;
use tantra_market_store::import::news_projection::NewsProjectionReader;
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
fn block_deals_reader_decodes_rows_and_nullable_remarks() {
    let dir = tempdir().unwrap();
    let block_deals_dir = dir.path().join("catalog").join("block_deals");
    std::fs::create_dir_all(&block_deals_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("trade_date", DataType::Date32, false),
        Field::new("exchange", DataType::Utf8, false),
        Field::new("instrument", DataType::Utf8, false),
        Field::new("security_name", DataType::Utf8, false),
        Field::new("client_name", DataType::Utf8, false),
        Field::new("side", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("quantity", DataType::Decimal128(18, 4), false),
        Field::new("price", DataType::Decimal128(18, 4), false),
        Field::new("remarks", DataType::Utf8, true),
    ]));
    let quantities = Decimal128Array::from(vec![Some(2_500_000_000_i128), Some(100_000_000_i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let prices = Decimal128Array::from(vec![Some(25_005_000_i128), Some(40_000_000_i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![0i32, 1])),
            Arc::new(StringArray::from(vec!["NSE", "NSE"])),
            Arc::new(StringArray::from(vec!["NSE:RELIANCE-EQ", "NSE:TCS-EQ"])),
            Arc::new(StringArray::from(vec!["RELIANCE", "TCS"])),
            Arc::new(StringArray::from(vec!["CLIENT A", "CLIENT B"])),
            Arc::new(StringArray::from(vec!["BUY", "SELL"])),
            Arc::new(StringArray::from(vec!["block", "bulk"])),
            Arc::new(quantities),
            Arc::new(prices),
            Arc::new(StringArray::from(vec![Some("Block deal window"), None])),
        ],
    )
    .unwrap();

    let path = block_deals_dir.join("2026-01-05.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = BlockDealsProjectionReader::new(dir.path()).read().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].trade_date,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
    );
    assert_eq!(rows[0].exchange, "NSE");
    assert_eq!(rows[0].instrument, "NSE:RELIANCE-EQ");
    assert_eq!(rows[0].security_name, "RELIANCE");
    assert_eq!(rows[0].client_name, "CLIENT A");
    assert_eq!(rows[0].side, "BUY");
    assert_eq!(rows[0].category, "block");
    assert_eq!(
        rows[0].quantity,
        Decimal::try_from_i128_with_scale(2_500_000_000, 4).unwrap()
    );
    assert_eq!(
        rows[0].price,
        Decimal::try_from_i128_with_scale(25_005_000, 4).unwrap()
    );
    assert_eq!(rows[0].remarks.as_deref(), Some("Block deal window"));
    assert_eq!(
        rows[1].trade_date,
        NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()
    );
    assert_eq!(rows[1].instrument, "NSE:TCS-EQ");
    assert_eq!(rows[1].side, "SELL");
    assert_eq!(rows[1].category, "bulk");
    assert_eq!(
        rows[1].quantity,
        Decimal::try_from_i128_with_scale(100_000_000, 4).unwrap()
    );
    assert_eq!(
        rows[1].price,
        Decimal::try_from_i128_with_scale(40_000_000, 4).unwrap()
    );
    assert_eq!(rows[1].remarks, None);
}

#[test]
fn news_reader_decodes_rows_and_nullable_summary() {
    let dir = tempdir().unwrap();
    let news_dir = dir.path().join("catalog").join("news");
    std::fs::create_dir_all(&news_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("published_date", DataType::Date32, false),
        Field::new("published_at", DataType::Utf8, false),
        Field::new("article_id", DataType::Utf8, false),
        Field::new("title", DataType::Utf8, false),
        Field::new("source", DataType::Utf8, false),
        Field::new("url", DataType::Utf8, false),
        Field::new("summary", DataType::Utf8, true),
        Field::new("instruments", DataType::Utf8, false),
    ]));
    let unix_epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let days_of = |y: i32, m: u32, d: u32| {
        i32::try_from((NaiveDate::from_ymd_opt(y, m, d).unwrap() - unix_epoch).num_days()).unwrap()
    };
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![
                days_of(2026, 1, 5),
                days_of(2026, 1, 6),
            ])),
            Arc::new(StringArray::from(vec![
                "2026-01-05T10:30:00+00:00",
                "2026-01-06T09:00:00Z",
            ])),
            Arc::new(StringArray::from(vec!["n1", "n2"])),
            Arc::new(StringArray::from(vec![
                "Reliance board meeting",
                "TCS results",
            ])),
            Arc::new(StringArray::from(vec!["moneycontrol", "moneycontrol"])),
            Arc::new(StringArray::from(vec![
                "https://example.com/reliance",
                "https://example.com/tcs",
            ])),
            Arc::new(StringArray::from(vec![Some("Q3 update"), None])),
            Arc::new(StringArray::from(vec![
                "NSE:RELIANCE-EQ",
                "NSE:TCS-EQ,NSE:INFY-EQ",
            ])),
        ],
    )
    .unwrap();

    let path = news_dir.join("2026-01-05.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = NewsProjectionReader::new(dir.path()).read().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].published_date,
        NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()
    );
    assert_eq!(
        rows[0].published_at,
        chrono::DateTime::parse_from_rfc3339("2026-01-05T10:30:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc)
    );
    assert_eq!(rows[0].article_id, "n1");
    assert_eq!(rows[0].title, "Reliance board meeting");
    assert_eq!(rows[0].source, "moneycontrol");
    assert_eq!(rows[0].url, "https://example.com/reliance");
    assert_eq!(rows[0].summary.as_deref(), Some("Q3 update"));
    assert_eq!(rows[0].instruments, vec!["NSE:RELIANCE-EQ"]);
    assert_eq!(
        rows[1].published_date,
        NaiveDate::from_ymd_opt(2026, 1, 6).unwrap()
    );
    assert_eq!(
        rows[1].published_at,
        chrono::DateTime::parse_from_rfc3339("2026-01-06T09:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    );
    assert_eq!(rows[1].article_id, "n2");
    assert_eq!(rows[1].title, "TCS results");
    assert_eq!(rows[1].summary, None);
    assert_eq!(rows[1].instruments, vec!["NSE:TCS-EQ", "NSE:INFY-EQ"]);
}

#[test]
fn institutional_flows_reader_decodes_rows_and_exact_decimals() {
    let dir = tempdir().unwrap();
    let flows_dir = dir.path().join("catalog").join("institutional_flows");
    std::fs::create_dir_all(&flows_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("trade_date", DataType::Date32, false),
        Field::new("participant", DataType::Utf8, false),
        Field::new("segment", DataType::Utf8, false),
        Field::new("gross_buy", DataType::Decimal128(18, 4), false),
        Field::new("gross_sell", DataType::Decimal128(18, 4), false),
        Field::new("net", DataType::Decimal128(18, 4), false),
        Field::new("source", DataType::Utf8, false),
    ]));
    let gross_buys =
        Decimal128Array::from(vec![Some(12_345_600_000_i128), Some(8_000_000_000_i128)])
            .with_precision_and_scale(18, 4)
            .unwrap();
    let gross_sells =
        Decimal128Array::from(vec![Some(10_000_000_000_i128), Some(7_500_000_000_i128)])
            .with_precision_and_scale(18, 4)
            .unwrap();
    let nets = Decimal128Array::from(vec![Some(2_345_600_000_i128), Some(500_000_000_i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![0i32, 1])),
            Arc::new(StringArray::from(vec!["FII", "DII"])),
            Arc::new(StringArray::from(vec!["cash", "cash"])),
            Arc::new(gross_buys),
            Arc::new(gross_sells),
            Arc::new(nets),
            Arc::new(StringArray::from(vec!["nse_bulk", "nse_bulk"])),
        ],
    )
    .unwrap();

    let path = flows_dir.join("2026-01-05.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = InstitutionalFlowsProjectionReader::new(dir.path())
        .read()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].trade_date,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
    );
    assert_eq!(rows[0].participant, "FII");
    assert_eq!(rows[0].segment, "cash");
    assert_eq!(
        rows[0].gross_buy,
        Decimal::try_from_i128_with_scale(12_345_600_000, 4).unwrap()
    );
    assert_eq!(
        rows[0].gross_sell,
        Decimal::try_from_i128_with_scale(10_000_000_000, 4).unwrap()
    );
    assert_eq!(
        rows[0].net,
        Decimal::try_from_i128_with_scale(2_345_600_000, 4).unwrap()
    );
    assert_eq!(rows[0].source, "nse_bulk");
    assert_eq!(
        rows[1].trade_date,
        NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()
    );
    assert_eq!(rows[1].participant, "DII");
    assert_eq!(
        rows[1].gross_buy,
        Decimal::try_from_i128_with_scale(8_000_000_000, 4).unwrap()
    );
    assert_eq!(
        rows[1].net,
        Decimal::try_from_i128_with_scale(500_000_000, 4).unwrap()
    );
}

#[test]
fn delivery_reader_decodes_rows_and_int64_quantities() {
    let dir = tempdir().unwrap();
    let delivery_dir = dir.path().join("catalog").join("delivery");
    std::fs::create_dir_all(&delivery_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("trade_date", DataType::Date32, false),
        Field::new("exchange", DataType::Utf8, false),
        Field::new("instrument", DataType::Utf8, false),
        Field::new("traded_quantity", DataType::Int64, false),
        Field::new("deliverable_quantity", DataType::Int64, false),
        Field::new("delivery_pct", DataType::Decimal128(18, 4), false),
    ]));
    let delivery_pcts = Decimal128Array::from(vec![Some(732_500_i128), Some(405_000_i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![0i32, 1])),
            Arc::new(StringArray::from(vec!["NSE", "NSE"])),
            Arc::new(StringArray::from(vec!["NSE:RELIANCE-EQ", "NSE:TCS-EQ"])),
            Arc::new(Int64Array::from(vec![1_234_567_i64, 890_123])),
            Arc::new(Int64Array::from(vec![904_000_i64, 360_500])),
            Arc::new(delivery_pcts),
        ],
    )
    .unwrap();

    let path = delivery_dir.join("2026-01-05.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = DeliveryProjectionReader::new(dir.path()).read().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].trade_date,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
    );
    assert_eq!(rows[0].exchange, "NSE");
    assert_eq!(rows[0].instrument, "NSE:RELIANCE-EQ");
    assert_eq!(rows[0].traded_quantity, 1_234_567);
    assert_eq!(rows[0].deliverable_quantity, 904_000);
    assert_eq!(
        rows[0].delivery_pct,
        Decimal::try_from_i128_with_scale(732_500, 4).unwrap()
    );
    assert_eq!(
        rows[1].trade_date,
        NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()
    );
    assert_eq!(rows[1].instrument, "NSE:TCS-EQ");
    assert_eq!(rows[1].traded_quantity, 890_123);
    assert_eq!(rows[1].deliverable_quantity, 360_500);
    assert_eq!(
        rows[1].delivery_pct,
        Decimal::try_from_i128_with_scale(405_000, 4).unwrap()
    );
}

#[test]
fn economic_calendar_reader_decodes_rows_and_nullable_columns() {
    let dir = tempdir().unwrap();
    let calendar_dir = dir.path().join("catalog").join("economic_calendar");
    std::fs::create_dir_all(&calendar_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("scheduled_date", DataType::Date32, false),
        Field::new("scheduled_at", DataType::Utf8, false),
        Field::new("instrument", DataType::Utf8, true),
        Field::new("country", DataType::Utf8, false),
        Field::new("currency", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("event_name", DataType::Utf8, false),
        Field::new("impact_level", DataType::Utf8, false),
        Field::new("actual", DataType::Decimal128(18, 4), true),
        Field::new("forecast", DataType::Decimal128(18, 4), true),
        Field::new("prior", DataType::Decimal128(18, 4), true),
        Field::new("period_label", DataType::Utf8, true),
    ]));
    let actuals = Decimal128Array::from(vec![Some(3_000_000_i128), None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let forecasts = Decimal128Array::from(vec![Some(2_900_000_i128), None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let priors = Decimal128Array::from(vec![Some(2_750_000_i128), None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![0i32, 1])),
            Arc::new(StringArray::from(vec![
                "2026-01-05T08:30:00Z",
                "2026-01-06T13:00:00Z",
            ])),
            Arc::new(StringArray::from(vec![Some("XAU-USD"), None])),
            Arc::new(StringArray::from(vec!["US", "IN"])),
            Arc::new(StringArray::from(vec!["USD", "INR"])),
            Arc::new(StringArray::from(vec!["Inflation", "GDP"])),
            Arc::new(StringArray::from(vec!["CPI m/m", "GDP y/y"])),
            Arc::new(StringArray::from(vec!["high", "medium"])),
            Arc::new(actuals),
            Arc::new(forecasts),
            Arc::new(priors),
            Arc::new(StringArray::from(vec![Some("Q4"), None])),
        ],
    )
    .unwrap();

    let path = calendar_dir.join("2026-01-05.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = EconomicCalendarProjectionReader::new(dir.path())
        .read()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].scheduled_date,
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
    );
    assert_eq!(rows[0].scheduled_at, "2026-01-05T08:30:00Z");
    assert_eq!(rows[0].instrument.as_deref(), Some("XAU-USD"));
    assert_eq!(rows[0].country, "US");
    assert_eq!(rows[0].currency, "USD");
    assert_eq!(rows[0].category, "Inflation");
    assert_eq!(rows[0].event_name, "CPI m/m");
    assert_eq!(rows[0].impact_level, "high");
    assert_eq!(
        rows[0].actual,
        Some(Decimal::try_from_i128_with_scale(3_000_000, 4).unwrap())
    );
    assert_eq!(
        rows[0].forecast,
        Some(Decimal::try_from_i128_with_scale(2_900_000, 4).unwrap())
    );
    assert_eq!(
        rows[0].prior,
        Some(Decimal::try_from_i128_with_scale(2_750_000, 4).unwrap())
    );
    assert_eq!(rows[0].period_label.as_deref(), Some("Q4"));
    assert_eq!(
        rows[1].scheduled_date,
        NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()
    );
    assert_eq!(rows[1].instrument, None);
    assert_eq!(rows[1].actual, None);
    assert_eq!(rows[1].forecast, None);
    assert_eq!(rows[1].prior, None);
    assert_eq!(rows[1].period_label, None);
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
    assert!(
        BlockDealsProjectionReader::new(dir.path())
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        InstitutionalFlowsProjectionReader::new(dir.path())
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        DeliveryProjectionReader::new(dir.path())
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        EconomicCalendarProjectionReader::new(dir.path())
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        NewsProjectionReader::new(dir.path())
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        FundamentalsProjectionReader::new(dir.path())
            .read_statements()
            .unwrap()
            .is_empty()
    );
    assert!(
        FundamentalsProjectionReader::new(dir.path())
            .read_ratios()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fundamentals_reader_decodes_statements_and_exact_decimals() {
    let dir = tempdir().unwrap();
    let fundamentals_dir = dir.path().join("catalog").join("fundamentals");
    std::fs::create_dir_all(&fundamentals_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("filing_date", DataType::Date32, false),
        Field::new("instrument", DataType::Utf8, false),
        Field::new("statement_type", DataType::Utf8, false),
        Field::new("period_type", DataType::Utf8, false),
        Field::new("period_end_date", DataType::Date32, false),
        Field::new("line_item", DataType::Utf8, false),
        Field::new("value", DataType::Decimal128(18, 4), false),
        Field::new("currency", DataType::Utf8, false),
    ]));
    // 2026-03-31 and 2026-11-14 as Unix-epoch day counts, computed not hardcoded.
    let epoch_days = |y: i32, m: u32, d: u32| -> i32 {
        (NaiveDate::from_ymd_opt(y, m, d).unwrap() - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as i32
    };
    let values = Decimal128Array::from(vec![1_057_219_000_000_000i128, 80_775_000_000_000])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 14); 2])),
            Arc::new(StringArray::from(vec![
                "NSE:RELIANCE-EQ",
                "NSE:RELIANCE-EQ",
            ])),
            Arc::new(StringArray::from(vec![
                "income_statement",
                "income_statement",
            ])),
            Arc::new(StringArray::from(vec!["annual", "annual"])),
            Arc::new(Date32Array::from(vec![epoch_days(2026, 3, 31); 2])),
            Arc::new(StringArray::from(vec!["total_revenue", "net_income"])),
            Arc::new(values),
            Arc::new(StringArray::from(vec!["INR", "INR"])),
        ],
    )
    .unwrap();

    let path = fundamentals_dir.join("2026-11-14.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = FundamentalsProjectionReader::new(dir.path())
        .read_statements()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].filing_date,
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
    );
    assert_eq!(rows[0].instrument, "NSE:RELIANCE-EQ");
    assert_eq!(rows[0].statement_type, "income_statement");
    assert_eq!(rows[0].period_type, "annual");
    assert_eq!(
        rows[0].period_end_date,
        NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()
    );
    assert_eq!(rows[0].line_item, "total_revenue");
    assert_eq!(
        rows[0].value,
        Decimal::try_from_i128_with_scale(1_057_219_000_000_000, 4).unwrap()
    );
    assert_eq!(rows[1].line_item, "net_income");
}

#[test]
fn fundamentals_reader_decodes_ratios_with_nullable_decimal_columns() {
    let dir = tempdir().unwrap();
    let ratios_dir = dir.path().join("catalog").join("fundamental_ratios");
    std::fs::create_dir_all(&ratios_dir).unwrap();

    let schema = Arc::new(Schema::new(vec![
        Field::new("as_of_date", DataType::Date32, false),
        Field::new("instrument", DataType::Utf8, false),
        Field::new("pe", DataType::Decimal128(18, 4), true),
        Field::new("pb", DataType::Decimal128(18, 4), true),
        Field::new("ev_to_ebitda", DataType::Decimal128(18, 4), true),
        Field::new("dividend_yield", DataType::Decimal128(18, 4), true),
        Field::new("roce", DataType::Decimal128(18, 4), true),
        Field::new("roe", DataType::Decimal128(18, 4), true),
        Field::new("debt_to_equity", DataType::Decimal128(18, 4), true),
        Field::new("interest_coverage", DataType::Decimal128(18, 4), true),
        Field::new("source", DataType::Utf8, false),
    ]));
    let epoch_days = |y: i32, m: u32, d: u32| -> i32 {
        (NaiveDate::from_ymd_opt(y, m, d).unwrap() - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as i32
    };
    let pe = Decimal128Array::from(vec![Some(22_078_114i128), None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let debt_equity = Decimal128Array::from(vec![Some(440_251i128), None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let nulls = Decimal128Array::from(vec![None, None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 14); 2])),
            Arc::new(StringArray::from(vec!["NSE:RELIANCE-EQ", "NSE:TCS"])),
            Arc::new(pe),
            Arc::new(nulls.clone()),
            Arc::new(nulls.clone()),
            Arc::new(nulls.clone()),
            Arc::new(nulls.clone()),
            Arc::new(nulls.clone()),
            Arc::new(debt_equity),
            Arc::new(nulls.clone()),
            Arc::new(StringArray::from(vec!["yfinance", "yfinance"])),
        ],
    )
    .unwrap();

    let path = ratios_dir.join("2026-11-14.parquet");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let rows = FundamentalsProjectionReader::new(dir.path())
        .read_ratios()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].as_of_date,
        NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()
    );
    assert_eq!(
        rows[0].pe,
        Some(Decimal::try_from_i128_with_scale(22_078_114, 4).unwrap())
    );
    assert_eq!(
        rows[0].debt_to_equity,
        Some(Decimal::try_from_i128_with_scale(440_251, 4).unwrap())
    );
    assert_eq!(rows[0].pb, None);
    assert_eq!(rows[1].pe, None);
}

#[test]
fn funds_projection_reader_decodes_meta_nav_and_returns() {
    let dir = tempdir().unwrap();
    let epoch_days = |y: i32, m: u32, d: u32| -> i32 {
        (NaiveDate::from_ymd_opt(y, m, d).unwrap() - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as i32
    };

    // Scheme metadata.
    let funds_dir = dir.path().join("catalog").join("mutual_funds");
    std::fs::create_dir_all(&funds_dir).unwrap();
    let funds_schema = Arc::new(Schema::new(vec![
        Field::new("as_of_date", DataType::Date32, false),
        Field::new("scheme_code", DataType::Utf8, false),
        Field::new("scheme_name", DataType::Utf8, false),
        Field::new("amc", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("isin_growth", DataType::Utf8, true),
        Field::new("isin_div_reinvestment", DataType::Utf8, true),
        Field::new("plan", DataType::Utf8, false),
        Field::new("option", DataType::Utf8, false),
        Field::new("source", DataType::Utf8, false),
    ]));
    let funds_batch = RecordBatch::try_new(
        funds_schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 14)])),
            Arc::new(StringArray::from(vec!["118565"])),
            Arc::new(StringArray::from(vec![
                "Parag Parikh Flexi Cap Fund - Direct Plan - Growth",
            ])),
            Arc::new(StringArray::from(vec!["PPFAS"])),
            Arc::new(StringArray::from(vec!["Flexi Cap"])),
            Arc::new(StringArray::from(vec![Some("INF175K01IM8")])),
            Arc::new(StringArray::from(vec![None::<&str>])),
            Arc::new(StringArray::from(vec!["Growth"])),
            Arc::new(StringArray::from(vec!["Direct"])),
            Arc::new(StringArray::from(vec!["amfi"])),
        ],
    )
    .unwrap();
    write_projection(
        &funds_dir.join("2026-11-14.parquet"),
        funds_schema,
        funds_batch,
    );

    // NAV observation.
    let nav_dir = dir.path().join("catalog").join("mf_nav");
    std::fs::create_dir_all(&nav_dir).unwrap();
    let nav_schema = Arc::new(Schema::new(vec![
        Field::new("nav_date", DataType::Date32, false),
        Field::new("scheme_code", DataType::Utf8, false),
        Field::new("nav", DataType::Decimal128(18, 4), false),
        Field::new("repurchase_price", DataType::Decimal128(18, 4), true),
        Field::new("sale_price", DataType::Decimal128(18, 4), true),
        Field::new("source", DataType::Utf8, false),
    ]));
    let nav = Decimal128Array::from(vec![10_455_190i128])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let prices = Decimal128Array::from(vec![None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let nav_batch = RecordBatch::try_new(
        nav_schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 13)])),
            Arc::new(StringArray::from(vec!["118565"])),
            Arc::new(nav),
            Arc::new(prices.clone()),
            Arc::new(prices.clone()),
            Arc::new(StringArray::from(vec!["amfi"])),
        ],
    )
    .unwrap();
    write_projection(&nav_dir.join("2026-11-13.parquet"), nav_schema, nav_batch);

    // Return metrics.
    let returns_dir = dir.path().join("catalog").join("mf_returns");
    std::fs::create_dir_all(&returns_dir).unwrap();
    let returns_schema = Arc::new(Schema::new(vec![
        Field::new("as_of_date", DataType::Date32, false),
        Field::new("scheme_code", DataType::Utf8, false),
        Field::new("cagr_1y", DataType::Decimal128(18, 4), true),
        Field::new("expense_ratio", DataType::Decimal128(18, 4), true),
        Field::new("source", DataType::Utf8, false),
    ]));
    let cagr = Decimal128Array::from(vec![Some(234_100i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let returns_batch = RecordBatch::try_new(
        returns_schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 14)])),
            Arc::new(StringArray::from(vec!["118565"])),
            Arc::new(cagr),
            Arc::new(prices.clone()),
            Arc::new(StringArray::from(vec!["amfi"])),
        ],
    )
    .unwrap();
    write_projection(
        &returns_dir.join("2026-11-14.parquet"),
        returns_schema,
        returns_batch,
    );

    let reader = FundsIposProjectionReader::new(dir.path());
    let metas = reader.read_mutual_funds().unwrap();
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].scheme_code, "118565");
    assert_eq!(metas[0].category, "Flexi Cap");
    assert_eq!(metas[0].isin_growth.as_deref(), Some("INF175K01IM8"));
    let navs = reader.read_mf_nav().unwrap();
    assert_eq!(
        navs[0].nav,
        Decimal::try_from_i128_with_scale(10_455_190, 4).unwrap()
    );
    assert_eq!(navs[0].repurchase_price, None);
    let returns = reader.read_mf_returns().unwrap();
    assert_eq!(
        returns[0].cagr_1y,
        Some(Decimal::try_from_i128_with_scale(234_100, 4).unwrap())
    );
    assert_eq!(returns[0].expense_ratio, None);
}

#[test]
fn ipo_projection_reader_decodes_issues_and_subscriptions() {
    let dir = tempdir().unwrap();
    let epoch_days = |y: i32, m: u32, d: u32| -> i32 {
        (NaiveDate::from_ymd_opt(y, m, d).unwrap() - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as i32
    };

    let ipos_dir = dir.path().join("catalog").join("ipos");
    std::fs::create_dir_all(&ipos_dir).unwrap();
    let ipos_schema = Arc::new(Schema::new(vec![
        Field::new("as_of_date", DataType::Date32, false),
        Field::new("symbol", DataType::Utf8, false),
        Field::new("company_name", DataType::Utf8, false),
        Field::new("board", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("open_date", DataType::Date32, true),
        Field::new("close_date", DataType::Date32, true),
        Field::new("listing_date", DataType::Date32, true),
        Field::new("price_band_low", DataType::Decimal128(18, 4), true),
        Field::new("price_band_high", DataType::Decimal128(18, 4), true),
        Field::new("issue_price", DataType::Decimal128(18, 4), true),
        Field::new("lot_size", DataType::Decimal128(18, 4), true),
        Field::new("issue_size_cr", DataType::Decimal128(18, 4), true),
        Field::new("source", DataType::Utf8, false),
    ]));
    let low = Decimal128Array::from(vec![Some(50_00000i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let high = Decimal128Array::from(vec![Some(55_00000i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let lot = Decimal128Array::from(vec![Some(2500i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let size = Decimal128Array::from(vec![Some(12_000_000i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let none = Decimal128Array::from(vec![None])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let ipos_batch = RecordBatch::try_new(
        ipos_schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 14)])),
            Arc::new(StringArray::from(vec!["ZAGGLECOM"])),
            Arc::new(StringArray::from(vec!["Zaggle Prepaid Ocean Services Ltd"])),
            Arc::new(StringArray::from(vec!["Main"])),
            Arc::new(StringArray::from(vec!["Open"])),
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 18)])),
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 20)])),
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 25)])),
            Arc::new(low),
            Arc::new(high),
            Arc::new(none.clone()),
            Arc::new(lot),
            Arc::new(size),
            Arc::new(StringArray::from(vec!["nse"])),
        ],
    )
    .unwrap();
    write_projection(
        &ipos_dir.join("2026-11-14.parquet"),
        ipos_schema,
        ipos_batch,
    );

    let subs_dir = dir.path().join("catalog").join("ipo_subscriptions");
    std::fs::create_dir_all(&subs_dir).unwrap();
    let subs_schema = Arc::new(Schema::new(vec![
        Field::new("as_of_date", DataType::Date32, false),
        Field::new("symbol", DataType::Utf8, false),
        Field::new("qib", DataType::Decimal128(18, 4), true),
        Field::new("nii", DataType::Decimal128(18, 4), true),
        Field::new("bnii", DataType::Decimal128(18, 4), true),
        Field::new("snii", DataType::Decimal128(18, 4), true),
        Field::new("rii", DataType::Decimal128(18, 4), true),
        Field::new("total", DataType::Decimal128(18, 4), true),
        Field::new("source", DataType::Utf8, false),
    ]));
    let qib = Decimal128Array::from(vec![Some(12_3450i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let total = Decimal128Array::from(vec![Some(28_9400i128)])
        .with_precision_and_scale(18, 4)
        .unwrap();
    let subs_batch = RecordBatch::try_new(
        subs_schema.clone(),
        vec![
            Arc::new(Date32Array::from(vec![epoch_days(2026, 11, 14)])),
            Arc::new(StringArray::from(vec!["ZAGGLECOM"])),
            Arc::new(qib),
            Arc::new(none.clone()),
            Arc::new(none.clone()),
            Arc::new(none.clone()),
            Arc::new(none.clone()),
            Arc::new(total),
            Arc::new(StringArray::from(vec!["nse"])),
        ],
    )
    .unwrap();
    write_projection(
        &subs_dir.join("2026-11-14.parquet"),
        subs_schema,
        subs_batch,
    );

    let reader = FundsIposProjectionReader::new(dir.path());
    let issues = reader.read_ipos().unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].symbol, "ZAGGLECOM");
    assert_eq!(issues[0].status, "Open");
    assert_eq!(
        issues[0].open_date,
        Some(NaiveDate::from_ymd_opt(2026, 11, 18).unwrap())
    );
    assert_eq!(
        issues[0].price_band_low,
        Some(Decimal::try_from_i128_with_scale(50_00000, 4).unwrap())
    );
    assert_eq!(issues[0].issue_price, None);
    let subscriptions = reader.read_ipo_subscriptions().unwrap();
    assert_eq!(
        subscriptions[0].qib,
        Some(Decimal::try_from_i128_with_scale(12_3450, 4).unwrap())
    );
    assert_eq!(
        subscriptions[0].total,
        Some(Decimal::try_from_i128_with_scale(28_9400, 4).unwrap())
    );
    assert_eq!(subscriptions[0].nii, None);
}

/// Writes one projection batch to `path` (test helper).
fn write_projection(path: &std::path::Path, schema: Arc<Schema>, batch: RecordBatch) {
    let file = std::fs::File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}
