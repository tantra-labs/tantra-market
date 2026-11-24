//! Integration tests for Lakehouse v3 manifests, snapshots, aliases, CAS compaction,
//! and strict `legacy_fallback` gating (`tantra-market/docs/gaps/0008`).

use std::fs::{self, File};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use arrow::array::{
    ArrayRef, Date32Array, Decimal128Array, Int32Array, LargeStringArray, StringArray,
};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use rust_decimal::Decimal;
use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity, Tick, Timeframe};
use tantra_market_store::OpenInterestStore;
use tantra_market_store::{
    BlockDealsProjectionReader, CorporateActionsProjectionReader, DeliveryProjectionReader,
    FilingsProjectionReader, FundamentalsProjectionReader, FundsIposProjectionReader,
    InstitutionalFlowsProjectionReader, Lakehouse, ManifestEntry, ManifestError, ManifestResolver,
    NewsProjectionReader, ParquetOpenInterestStore, RecorderConfig, TickRecorder, commit_entries,
    create_tag, write_cas_batch,
};
use tempfile::tempdir;

const UNIX_EPOCH_DAYS_FROM_CE: i64 = 719_163;

fn utc(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, hh, mm, 0).unwrap()
}

fn date32_days(d: NaiveDate) -> i32 {
    use chrono::Datelike;
    (i64::from(d.num_days_from_ce()) - UNIX_EPOCH_DAYS_FROM_CE) as i32
}

fn make_bar(inst: &InstrumentId, ts: DateTime<Utc>, close: &str) -> Bar {
    let p = Price::try_new(close.parse::<Decimal>().unwrap()).unwrap();
    Bar {
        instrument: inst.clone(),
        open: p,
        high: Price::try_new(p.value() + Decimal::ONE).unwrap(),
        low: Price::try_new(p.value() - Decimal::ONE).unwrap(),
        close: p,
        volume: Quantity::try_new(Decimal::from(1000)).unwrap(),
        timestamp: ts,
    }
}

fn write_large_aliases_cas(root: &Path) -> String {
    let schema = Arc::new(Schema::new(vec![
        Field::new("exchange", DataType::LargeUtf8, false),
        Field::new("symbol", DataType::LargeUtf8, false),
        Field::new("series", DataType::LargeUtf8, true),
        Field::new("valid_from", DataType::Date32, true),
        Field::new("valid_to", DataType::Date32, true),
        Field::new("instrument_uid", DataType::LargeUtf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(LargeStringArray::from(vec![
                Some("NSE"),
                Some("NSE"),
                Some("NSE"),
            ])),
            Arc::new(LargeStringArray::from(vec![
                Some("ZOMATO"),
                Some("ETERNAL"),
                Some("RELIANCE"),
            ])),
            Arc::new(LargeStringArray::from(vec![
                Some("EQ"),
                Some("EQ"),
                Some("EQ"),
            ])),
            Arc::new(Date32Array::from(vec![
                Some(date32_days(NaiveDate::from_ymd_opt(2021, 7, 23).unwrap())),
                Some(date32_days(NaiveDate::from_ymd_opt(2025, 4, 9).unwrap())),
                None,
            ])),
            Arc::new(Date32Array::from(vec![
                Some(date32_days(NaiveDate::from_ymd_opt(2025, 4, 8).unwrap())),
                None,
                None,
            ])),
            Arc::new(LargeStringArray::from(vec![
                Some("INE758T01015"),
                Some("INE758T01015"),
                Some("INE002A01018"),
            ])),
        ],
    )
    .unwrap();
    write_cas_batch(root, &batch, Compression::SNAPPY).unwrap()
}

#[tokio::test]
async fn compact_v3_and_pinning_preserves_tags_and_legacy_fallback_gate() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();

    let reliance = InstrumentId::try_new(Exchange::Nse, "RELIANCE").unwrap();
    let lh = Lakehouse::new(root);

    // Phase 1: write initial bar + live ticks across multi-segment flushes via TickRecorder.
    let bar_v1 = make_bar(&reliance, utc(2026, 9, 1, 9, 45), "2500.0000");
    lh.write_bars(Timeframe::D1, &[bar_v1]).unwrap();

    let recorder = TickRecorder::spawn(
        lh.clone(),
        RecorderConfig {
            max_ticks: 1,
            flush_interval: Duration::from_secs(60),
            ..Default::default()
        },
    );
    assert!(
        recorder
            .record(Tick::Ltp {
                instrument: reliance.clone(),
                price: Price::try_new("2500.2500".parse().unwrap()).unwrap(),
                volume: Quantity::try_new(Decimal::from(10)).unwrap(),
                exchange_ts: utc(2026, 9, 1, 4, 0),
                receive_ts: utc(2026, 9, 1, 4, 0),
            })
            .await
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        recorder
            .record(Tick::Ltp {
                instrument: reliance.clone(),
                price: Price::try_new("2501.5000".parse().unwrap()).unwrap(),
                volume: Quantity::try_new(Decimal::from(20)).unwrap(),
                exchange_ts: utc(2026, 9, 1, 4, 1),
                receive_ts: utc(2026, 9, 1, 4, 1),
            })
            .await
    );
    let stats = recorder.shutdown().await.unwrap();
    assert_eq!(stats.written, 2);

    // Compact into v3 CAS commit #1 and pin refs/tags/v1.
    let report1 = lh.compact_v3(true).unwrap();
    assert!(report1.segments_merged >= 2);
    assert!(report1.rows_written >= 3);
    create_tag(root, "v1", &report1.commit_sha).unwrap();

    // Verify Hive segments were removed and strict v3 reader still sees everything via CAS.
    let strict_v1 = Lakehouse::new(root).with_legacy_fallback(false);
    let bars_v1 = strict_v1
        .read_bars_range(
            &reliance,
            Timeframe::D1,
            utc(2026, 9, 1, 0, 0),
            utc(2026, 9, 2, 0, 0),
        )
        .unwrap();
    assert_eq!(bars_v1.len(), 1);
    assert_eq!(
        bars_v1[0].close.value(),
        "2500.0000".parse::<Decimal>().unwrap()
    );

    let ticks_v1 = strict_v1
        .read_ticks_range(&reliance, utc(2026, 9, 1, 0, 0), utc(2026, 9, 2, 0, 0))
        .unwrap();
    assert_eq!(ticks_v1.len(), 2);

    // Phase 2: write additional bar in another month and compact again -> advances main, leaves v1 untouched.
    let bar_v2 = make_bar(&reliance, utc(2026, 10, 1, 9, 45), "2610.0000");
    lh.write_bars(Timeframe::D1, &[bar_v2]).unwrap();
    let report2 = lh.compact_v3(true).unwrap();
    assert_ne!(report2.commit_sha, report1.commit_sha);

    let main_bars = strict_v1
        .read_bars_range(
            &reliance,
            Timeframe::D1,
            utc(2026, 8, 1, 0, 0),
            utc(2026, 11, 1, 0, 0),
        )
        .unwrap();
    assert_eq!(main_bars.len(), 2);

    let pinned_v1 = Lakehouse::new(root)
        .with_legacy_fallback(false)
        .with_ref("refs/tags/v1");
    let pinned_bars = pinned_v1
        .read_bars_range(
            &reliance,
            Timeframe::D1,
            utc(2026, 8, 1, 0, 0),
            utc(2026, 11, 1, 0, 0),
        )
        .unwrap();
    assert_eq!(pinned_bars.len(), 1);
    assert_eq!(pinned_bars[0].timestamp, utc(2026, 9, 1, 9, 45));

    // Also resolve directly by raw 64-hex commit SHA.
    let by_sha = Lakehouse::new(root)
        .with_legacy_fallback(false)
        .with_ref(&report1.commit_sha);
    assert_eq!(
        by_sha
            .read_bars_range(
                &reliance,
                Timeframe::D1,
                utc(2026, 8, 1, 0, 0),
                utc(2026, 11, 1, 0, 0)
            )
            .unwrap()
            .len(),
        1
    );

    // Phase 3: write an uncompacted Hive segment; strict v3 ignores it, legacy fallback sees it.
    let tcs = InstrumentId::try_new(Exchange::Nse, "TCS").unwrap();
    lh.write_bars(
        Timeframe::D1,
        &[make_bar(&tcs, utc(2026, 10, 2, 9, 45), "3900.0000")],
    )
    .unwrap();
    assert!(
        strict_v1
            .read_bars_range(
                &tcs,
                Timeframe::D1,
                utc(2026, 10, 1, 0, 0),
                utc(2026, 10, 5, 0, 0)
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        Lakehouse::new(root)
            .read_bars_range(
                &tcs,
                Timeframe::D1,
                utc(2026, 10, 1, 0, 0),
                utc(2026, 10, 5, 0, 0)
            )
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn resolver_aliases_large_utf8_and_missing_tree_rules() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();

    let alias_sha = write_large_aliases_cas(root);
    let d1 = NaiveDate::from_ymd_opt(2025, 4, 10).unwrap();
    let ts = d1.and_hms_opt(12, 0, 0).unwrap().and_utc();
    let commit = commit_entries(
        root,
        "add aliases",
        d1,
        vec![ManifestEntry {
            domain: "reference".into(),
            dataset: "aliases".into(),
            exchange: Some("NSE".into()),
            asset_class: None,
            timeframe: None,
            time_bucket: "snapshot=2025".into(),
            object_sha: alias_sha,
            min_ts: ts,
            max_ts: ts,
            row_count: 3,
            sort_key: "symbol".into(),
        }],
        None,
        None,
    )
    .unwrap();
    create_tag(root, "alias-v1", &commit.commit_sha).unwrap();

    let resolver = ManifestResolver::new(root);
    let aliases = resolver.load_aliases(Some("refs/tags/alias-v1")).unwrap();
    assert_eq!(aliases.len(), 3);
    assert_eq!(
        ManifestResolver::resolve_alias(
            &aliases,
            "NSE",
            "ZOMATO",
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
        )
        .as_deref(),
        Some("INE758T01015")
    );
    assert_eq!(
        ManifestResolver::resolve_alias(
            &aliases,
            "NSE",
            "ETERNAL",
            Some(NaiveDate::from_ymd_opt(2025, 5, 1).unwrap()),
        )
        .as_deref(),
        Some("INE758T01015")
    );
    assert_eq!(
        ManifestResolver::canonical_symbol(&aliases, "INE758T01015", None),
        Some((Exchange::Nse, "ETERNAL".to_string()))
    );

    // If tree JSON is deleted, views/ fallback works ONLY for main when views/ exists,
    // NEVER for a pinned tag (must error with MissingTree).
    let tree_file = root
        .join("trees")
        .join(format!("{}.json", commit.tree_hash));
    fs::remove_file(&tree_file).unwrap();

    // Pinned tag fails with MissingTree even if views/ exists.
    let view_dir =
        root.join("views/market/bars/exchange=NSE/asset_class=equity/timeframe=1d/year=2026");
    fs::create_dir_all(&view_dir).unwrap();
    let err = resolver
        .load_entries(Some("refs/tags/alias-v1"))
        .unwrap_err();
    assert!(matches!(err, ManifestError::MissingTree { .. }));

    // Unpinned main succeeds by scanning views/.
    let (main_commit, _entries) = resolver.load_entries(None).unwrap();
    assert_eq!(main_commit.commit_sha, commit.commit_sha);
}

#[tokio::test]
async fn open_interest_and_projection_readers_read_v3_cas_with_large_utf8() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let mut entries = Vec::new();
    let day = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let day_num = date32_days(day);
    let ts = day.and_hms_opt(10, 0, 0).unwrap().and_utc();

    let dec_arr = |vals: Vec<Option<i128>>| -> ArrayRef {
        Arc::new(
            Decimal128Array::from(vals)
                .with_precision_and_scale(18, 4)
                .unwrap(),
        )
    };
    let lstr = |vals: Vec<Option<&str>>| -> ArrayRef { Arc::new(LargeStringArray::from(vals)) };
    let str_arr = |vals: Vec<Option<&str>>| -> ArrayRef { Arc::new(StringArray::from(vals)) };
    let d32 = |vals: Vec<Option<i32>>| -> ArrayRef { Arc::new(Date32Array::from(vals)) };

    // 1. derivatives/open_interest with instrument_uid + LargeUtf8 + Date32
    {
        let schema = Arc::new(Schema::new(vec![
            Field::new("instrument_uid", DataType::LargeUtf8, false),
            Field::new("as_of", DataType::Date32, false),
            Field::new("open_interest", DataType::Decimal128(18, 4), false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                lstr(vec![Some("NFO:NIFTY26SEPFUT")]),
                d32(vec![Some(day_num)]),
                dec_arr(vec![Some(123_450_000)]),
            ],
        )
        .unwrap();
        let sha = write_cas_batch(root, &batch, Compression::ZSTD(Default::default())).unwrap();
        entries.push(ManifestEntry {
            domain: "derivatives".into(),
            dataset: "open_interest".into(),
            exchange: Some("NFO".into()),
            asset_class: Some("derivatives".into()),
            timeframe: None,
            time_bucket: "date=2026-09-15".into(),
            object_sha: sha,
            min_ts: ts,
            max_ts: ts,
            row_count: 1,
            sort_key: "instrument_uid".into(),
        });
    }

    // 2. disclosures/block_deals
    {
        let schema = Arc::new(Schema::new(vec![
            Field::new("trade_date", DataType::Date32, false),
            Field::new("exchange", DataType::LargeUtf8, false),
            Field::new("instrument", DataType::LargeUtf8, false),
            Field::new("security_name", DataType::LargeUtf8, false),
            Field::new("client_name", DataType::LargeUtf8, false),
            Field::new("side", DataType::LargeUtf8, false),
            Field::new("category", DataType::LargeUtf8, false),
            Field::new("quantity", DataType::Decimal128(18, 4), false),
            Field::new("price", DataType::Decimal128(18, 4), false),
            Field::new("remarks", DataType::LargeUtf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                d32(vec![Some(day_num)]),
                lstr(vec![Some("NSE")]),
                lstr(vec![Some("NSE:RELIANCE")]),
                lstr(vec![Some("Reliance Industries")]),
                lstr(vec![Some("Alpha Fund")]),
                lstr(vec![Some("BUY")]),
                lstr(vec![Some("block")]),
                dec_arr(vec![Some(5_000_000)]),
                dec_arr(vec![Some(25_005_000)]),
                lstr(vec![Some("v3 CAS")]),
            ],
        )
        .unwrap();
        let sha = write_cas_batch(root, &batch, Compression::ZSTD(Default::default())).unwrap();
        entries.push(ManifestEntry {
            domain: "disclosures".into(),
            dataset: "block_deals".into(),
            exchange: Some("NSE".into()),
            asset_class: None,
            timeframe: None,
            time_bucket: "date=2026-09-15".into(),
            object_sha: sha,
            min_ts: ts,
            max_ts: ts,
            row_count: 1,
            sort_key: "trade_date".into(),
        });
    }

    // 3. fundamentals/filings & fundamentals/corporate_actions
    {
        let schema = Arc::new(Schema::new(vec![
            Field::new("filing_date", DataType::Date32, false),
            Field::new("instrument", DataType::LargeUtf8, false),
            Field::new("filing_type", DataType::LargeUtf8, false),
            Field::new("title", DataType::LargeUtf8, false),
            Field::new("source_url", DataType::LargeUtf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                d32(vec![Some(day_num)]),
                lstr(vec![Some("NSE:RELIANCE")]),
                lstr(vec![Some("quarterly_result")]),
                lstr(vec![Some("Q2 FY27")]),
                lstr(vec![None]),
            ],
        )
        .unwrap();
        let sha = write_cas_batch(root, &batch, Compression::ZSTD(Default::default())).unwrap();
        entries.push(ManifestEntry {
            domain: "fundamentals".into(),
            dataset: "filings".into(),
            exchange: Some("NSE".into()),
            asset_class: None,
            timeframe: None,
            time_bucket: "date=2026-09-15".into(),
            object_sha: sha,
            min_ts: ts,
            max_ts: ts,
            row_count: 1,
            sort_key: "filing_date".into(),
        });

        let ca_schema = Arc::new(Schema::new(vec![
            Field::new("ex_date", DataType::Date32, false),
            Field::new("instrument", DataType::Utf8, false),
            Field::new("action_type", DataType::Utf8, false),
            Field::new("old_shares", DataType::Int32, true),
            Field::new("new_shares", DataType::Int32, true),
            Field::new("per_share", DataType::Decimal128(18, 4), true),
            Field::new("issue_price", DataType::Decimal128(18, 4), true),
            Field::new("currency", DataType::Utf8, false),
        ]));
        let ca_batch = RecordBatch::try_new(
            ca_schema,
            vec![
                d32(vec![Some(day_num)]),
                str_arr(vec![Some("NSE:RELIANCE")]),
                str_arr(vec![Some("dividend")]),
                Arc::new(Int32Array::from(vec![None])),
                Arc::new(Int32Array::from(vec![None])),
                dec_arr(vec![Some(100_000)]),
                dec_arr(vec![None]),
                str_arr(vec![Some("INR")]),
            ],
        )
        .unwrap();
        let ca_sha =
            write_cas_batch(root, &ca_batch, Compression::ZSTD(Default::default())).unwrap();
        entries.push(ManifestEntry {
            domain: "fundamentals".into(),
            dataset: "corporate_actions".into(),
            exchange: Some("NSE".into()),
            asset_class: None,
            timeframe: None,
            time_bucket: "date=2026-09-15".into(),
            object_sha: ca_sha,
            min_ts: ts,
            max_ts: ts,
            row_count: 1,
            sort_key: "ex_date".into(),
        });
    }

    // 4. Commit everything and read back strictly (legacy_fallback = false).
    let commit = commit_entries(root, "projections v3", day, entries, None, None).unwrap();
    create_tag(root, "proj-v1", &commit.commit_sha).unwrap();

    // Also place a conflicting file in legacy catalog/block_deals to prove legacy_fallback(false) ignores it.
    let legacy_bd = root.join("catalog/block_deals");
    fs::create_dir_all(&legacy_bd).unwrap();
    let dummy_file = File::create(legacy_bd.join("2026-09-15.parquet")).unwrap();
    let bd_schema = Arc::new(Schema::new(vec![Field::new("bad", DataType::Int32, false)]));
    let bad_batch =
        RecordBatch::try_new(bd_schema.clone(), vec![Arc::new(Int32Array::from(vec![1]))]).unwrap();
    let mut w = ArrowWriter::try_new(dummy_file, bd_schema, None).unwrap();
    w.write(&bad_batch).unwrap();
    w.close().unwrap();

    let oi_store =
        ParquetOpenInterestStore::open_with_options(root, Some("refs/tags/proj-v1"), false)
            .unwrap();
    let fut = InstrumentId::try_new(Exchange::Nfo, "NIFTY26SEPFUT").unwrap();
    let snap = oi_store.latest(&fut, None).await.unwrap().unwrap();
    assert_eq!(snap.open_interest.value(), Decimal::from(12345));

    let deals = BlockDealsProjectionReader::new(root)
        .with_ref("refs/tags/proj-v1")
        .with_legacy_fallback(false)
        .read()
        .unwrap();
    assert_eq!(deals.len(), 1);
    assert_eq!(deals[0].remarks.as_deref(), Some("v3 CAS"));

    let filings = FilingsProjectionReader::new(root)
        .with_ref("refs/tags/proj-v1")
        .with_legacy_fallback(false)
        .read()
        .unwrap();
    assert_eq!(filings.len(), 1);
    assert_eq!(filings[0].title, "Q2 FY27");

    let cas = CorporateActionsProjectionReader::new(root)
        .with_ref("refs/tags/proj-v1")
        .with_legacy_fallback(false)
        .read()
        .unwrap();
    assert_eq!(cas.len(), 1);
    assert_eq!(cas[0].per_share, Some(Decimal::from(10)));

    // Empty projection datasets return Ok(empty) without errors when missing in v3 manifest.
    assert!(
        DeliveryProjectionReader::new(root)
            .with_legacy_fallback(false)
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        InstitutionalFlowsProjectionReader::new(root)
            .with_legacy_fallback(false)
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        NewsProjectionReader::new(root)
            .with_legacy_fallback(false)
            .read()
            .unwrap()
            .is_empty()
    );
    assert!(
        FundamentalsProjectionReader::new(root)
            .with_legacy_fallback(false)
            .read_statements()
            .unwrap()
            .is_empty()
    );
    assert!(
        FundsIposProjectionReader::new(root)
            .with_legacy_fallback(false)
            .read_ipos()
            .unwrap()
            .is_empty()
    );
}
