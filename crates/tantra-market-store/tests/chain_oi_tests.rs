//! Integration tests for the T3.1/T3.2 work: options-chain enumeration and reader
//! (`docs/gaps/0001`), and open-interest snapshot stores + four-quadrant buildup
//! classification (`docs/gaps/0002`). Fixtures are generated in temp dirs; nothing touches
//! the network.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, StringArray};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use chrono::NaiveDate;
use parquet::arrow::ArrowWriter;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tantra_core::{Exchange, InstrumentId, OptionType, Quantity};
use tantra_market_store::ports::{
    BarReader, BarRequest, ChainReader, InstrumentMaster, PortError, QuoteReader,
};
use tantra_market_store::wire::{Bar, BarType, UnixNanos};
use tantra_market_store::{
    BuildupAnalyzer, BuildupClass, BuildupReader, CatalogInstrumentMaster, ChainContract,
    InMemoryOpenInterestStore, OpenInterestSnapshot, OpenInterestStore, ParquetOpenInterestStore,
    classify_buildup, snapshot_schema, write_oi_file,
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tantra_chain_oi_it_{}_{name}", std::process::id()));
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

/// One option row for the catalog fixture.
struct OptRow {
    symbol: &'static str,
    expiry: &'static str,
    strike: f64,
    option_type: &'static str,
    underlying: &'static str,
    lot_size: f64,
}

/// Writes `{root}/instruments/{date}.parquet` with the option rows.
fn write_catalog(root: &Path, date: &str, rows: &[OptRow]) {
    let dir = root.join("instruments");
    std::fs::create_dir_all(&dir).unwrap();
    let strings = |values: Vec<Option<&str>>| -> ArrayRef { Arc::new(StringArray::from(values)) };
    let numbers = |values: Vec<Option<f64>>| -> ArrayRef { Arc::new(Float64Array::from(values)) };
    let columns: Vec<ArrayRef> = vec![
        strings(rows.iter().map(|r| Some(r.symbol)).collect()),
        strings(rows.iter().map(|_| Some("NFO")).collect()),
        strings(rows.iter().map(|_| Some("option")).collect()),
        strings(rows.iter().map(|_| Some("INR")).collect()),
        numbers(rows.iter().map(|r| Some(r.lot_size)).collect()),
        numbers(rows.iter().map(|_| Some(0.05)).collect()),
        numbers(rows.iter().map(|_| Some(1500.0)).collect()),
        strings(rows.iter().map(|r| Some(r.expiry)).collect()),
        numbers(rows.iter().map(|r| Some(r.strike)).collect()),
        strings(rows.iter().map(|r| Some(r.option_type)).collect()),
        strings(rows.iter().map(|r| Some(r.underlying)).collect()),
        strings(rows.iter().map(|_| None).collect()),
    ];
    let schema = Arc::new(snapshot_schema());
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    let file = File::create(dir.join(format!("{date}.parquet"))).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn option_rows() -> Vec<OptRow> {
    vec![
        OptRow {
            symbol: "NIFTY24OCT24500CE",
            expiry: "2024-10-31",
            strike: 24500.0,
            option_type: "CE",
            underlying: "NIFTY",
            lot_size: 25.0,
        },
        OptRow {
            symbol: "NIFTY24OCT24000CE",
            expiry: "2024-10-31",
            strike: 24000.0,
            option_type: "CE",
            underlying: "NIFTY",
            lot_size: 25.0,
        },
        OptRow {
            symbol: "NIFTY24OCT24000PE",
            expiry: "2024-10-31",
            strike: 24000.0,
            option_type: "PE",
            underlying: "NIFTY",
            lot_size: 25.0,
        },
        OptRow {
            symbol: "NIFTY24NOV24000CE",
            expiry: "2024-11-28",
            strike: 24000.0,
            option_type: "CE",
            underlying: "NIFTY",
            lot_size: 25.0,
        },
        OptRow {
            symbol: "BANKNIFTY24OCT24000CE",
            expiry: "2024-10-31",
            strike: 24000.0,
            option_type: "CE",
            underlying: "BANKNIFTY",
            lot_size: 15.0,
        },
    ]
}

fn nifty() -> InstrumentId {
    InstrumentId::new(Exchange::Nfo, "NIFTY")
}

fn contract(symbol: &str) -> InstrumentId {
    InstrumentId::new(Exchange::Nfo, symbol)
}

// ——————————————————————————————————————————————————————————————————————————
// T3.1 — chain enumeration on the instrument master
// ——————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn list_chain_filters_underlying_and_orders_by_expiry_then_strike_then_type() {
    let dir = TempDir::new("chain_order");
    write_catalog(&dir.0, "2024-10-01", &option_rows());
    let master = CatalogInstrumentMaster::open(&dir.0).unwrap();

    let chain = master.list_chain(&nifty(), None).await.unwrap();
    let symbols: Vec<&str> = chain.iter().map(|i| i.id().symbol.as_str()).collect();
    assert_eq!(
        symbols,
        vec![
            "NIFTY24OCT24000CE",
            "NIFTY24OCT24000PE",
            "NIFTY24OCT24500CE",
            "NIFTY24NOV24000CE",
        ],
        "BANKNIFTY contracts must be excluded; ordering must be expiry, strike, then CE<PE"
    );
    assert_eq!(chain[0].strike(), Some(24000.0));
    assert_eq!(chain[0].option_type(), Some(OptionType::Call));
}

#[tokio::test]
async fn list_chain_as_of_excludes_expired_contracts() {
    let dir = TempDir::new("chain_as_of");
    write_catalog(&dir.0, "2024-10-01", &option_rows());
    let master = CatalogInstrumentMaster::open(&dir.0).unwrap();

    let as_of = NaiveDate::from_ymd_opt(2024, 11, 15).unwrap();
    let chain = master.list_chain(&nifty(), Some(as_of)).await.unwrap();
    let symbols: Vec<&str> = chain.iter().map(|i| i.id().symbol.as_str()).collect();
    assert_eq!(symbols, vec!["NIFTY24NOV24000CE"]);
}

#[tokio::test]
async fn default_list_chain_is_unsupported() {
    struct NoChain;
    #[async_trait]
    impl InstrumentMaster for NoChain {
        async fn get_instrument(
            &self,
            _id: &InstrumentId,
        ) -> Result<Option<tantra_market_store::wire::Instrument>, PortError> {
            Ok(None)
        }
        async fn list_instruments(
            &self,
        ) -> Result<Vec<tantra_market_store::wire::Instrument>, PortError> {
            Ok(Vec::new())
        }
    }
    let err = NoChain.list_chain(&nifty(), None).await.unwrap_err();
    assert!(matches!(err, PortError::Unsupported(_)));
}

// ——————————————————————————————————————————————————————————————————————————
// T3.1 — chain reader join with quotes and OI
// ——————————————————————————————————————————————————————————————————————————

struct FakeQuoteReader {
    quotes: std::collections::HashMap<InstrumentId, (f64, f64)>,
}

#[async_trait]
impl QuoteReader for FakeQuoteReader {
    async fn read_quote(
        &self,
        id: &InstrumentId,
        _as_of: Option<UnixNanos>,
    ) -> Result<Option<tantra_market_store::wire::QuoteTick>, PortError> {
        Ok(self.quotes.get(id).map(|(bid, ask)| {
            tantra_market_store::wire::QuoteTick::new(
                id.clone(),
                *bid,
                *ask,
                0.0,
                0.0,
                UnixNanos::from_u64(0),
                UnixNanos::from_u64(0),
            )
        }))
    }
}

#[tokio::test]
async fn chain_reader_joins_quotes_and_oi_and_applies_filters() {
    let dir = TempDir::new("chain_reader");
    write_catalog(&dir.0, "2024-10-01", &option_rows());
    let master = Arc::new(CatalogInstrumentMaster::open(&dir.0).unwrap());

    let quotes = Arc::new(FakeQuoteReader {
        quotes: std::collections::HashMap::from([
            (contract("NIFTY24OCT24000CE"), (235.0, 236.0)),
            (contract("NIFTY24OCT24000PE"), (100.0, 101.0)),
        ]),
    });
    let oi = Arc::new(InMemoryOpenInterestStore::new());
    oi.put(OpenInterestSnapshot {
        instrument: contract("NIFTY24OCT24000CE"),
        as_of: NaiveDate::from_ymd_opt(2024, 10, 1).unwrap(),
        open_interest: Quantity::new(Decimal::from(1_250_000)),
    })
    .await
    .unwrap();

    let reader = tantra_market_store::InstrumentChainReader::new(master.clone(), quotes.clone())
        .with_oi_store(oi.clone());

    // No filters: full chain, quotes joined, OI only where the store has it.
    let chain = reader.options_chain(&nifty(), None, None).await.unwrap();
    assert_eq!(chain.len(), 4);
    assert_eq!(chain[0].instrument, contract("NIFTY24OCT24000CE"));
    assert_eq!(
        chain[0].bid,
        Some(tantra_core::Price::new(Decimal::from(235)))
    );
    assert_eq!(
        chain[0].ask,
        Some(tantra_core::Price::new(Decimal::from(236)))
    );
    assert_eq!(
        chain[0].open_interest,
        Some(Quantity::new(Decimal::from(1_250_000)))
    );
    assert_eq!(
        chain[1].open_interest, None,
        "no OI snapshot for the 24000PE"
    );
    assert_eq!(
        chain[0].expiry,
        NaiveDate::from_ymd_opt(2024, 10, 31).unwrap()
    );
    assert_eq!(chain[0].lot_size, Quantity::new(Decimal::from(25)));
    assert_eq!(chain[0].option_type, OptionType::Call);

    // Expiry filter keeps only the November contract.
    let expiry = NaiveDate::from_ymd_opt(2024, 11, 28).unwrap();
    let filtered = reader
        .options_chain(&nifty(), Some(expiry), None)
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].instrument, contract("NIFTY24NOV24000CE"));

    // Limit caps the result.
    let capped = reader.options_chain(&nifty(), None, Some(2)).await.unwrap();
    assert_eq!(capped.len(), 2);
}

#[tokio::test]
async fn chain_reader_without_oi_store_leaves_open_interest_none() {
    let dir = TempDir::new("chain_reader_no_oi");
    write_catalog(&dir.0, "2024-10-01", &option_rows());
    let master = Arc::new(CatalogInstrumentMaster::open(&dir.0).unwrap());
    let reader = tantra_market_store::InstrumentChainReader::new(
        master,
        Arc::new(FakeQuoteReader {
            quotes: std::collections::HashMap::new(),
        }),
    );
    let chain: Vec<ChainContract> = reader.options_chain(&nifty(), None, None).await.unwrap();
    assert!(chain.iter().all(|c| c.open_interest.is_none()));
    assert!(chain.iter().all(|c| c.bid.is_none() && c.ask.is_none()));
}

// ——————————————————————————————————————————————————————————————————————————
// T3.2 — OI snapshot stores
// ——————————————————————————————————————————————————————————————————————————

#[tokio::test]
async fn in_memory_oi_store_history_and_latest() {
    let store = InMemoryOpenInterestStore::new();
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    let inst = contract("NIFTY24OCT24000CE");
    for (day, oi) in [(1, 1000_i64), (2, 1100), (3, 900)] {
        store
            .put(OpenInterestSnapshot {
                instrument: inst.clone(),
                as_of: d(2024, 10, day),
                open_interest: Quantity::new(Decimal::from(oi)),
            })
            .await
            .unwrap();
    }
    // A later put on the same day replaces.
    store
        .put(OpenInterestSnapshot {
            instrument: inst.clone(),
            as_of: d(2024, 10, 3),
            open_interest: Quantity::new(Decimal::from(950)),
        })
        .await
        .unwrap();

    let history = store
        .history(&inst, d(2024, 10, 2), Some(d(2024, 10, 3)))
        .await
        .unwrap();
    let values: Vec<i64> = history
        .iter()
        .map(|s| s.open_interest.0.to_i64().unwrap())
        .collect();
    assert_eq!(values, vec![1100, 950]);

    let latest = store
        .latest(&inst, Some(d(2024, 10, 2)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.open_interest.0.to_i64().unwrap(), 1100);
    let overall = store.latest(&inst, None).await.unwrap().unwrap();
    assert_eq!(overall.as_of, d(2024, 10, 3));
    assert_eq!(overall.open_interest.0.to_i64().unwrap(), 950);
    assert!(
        store
            .latest(&contract("OTHER"), None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn parquet_oi_store_round_trips_exact_quantities() {
    let dir = TempDir::new("oi_parquet");
    let inst = contract("NIFTY24OCT24000CE");
    let other = contract("NIFTY24OCT24000PE");
    let date = NaiveDate::from_ymd_opt(2024, 10, 1).unwrap();
    write_oi_file(
        &dir.0,
        date,
        &[
            OpenInterestSnapshot {
                instrument: inst.clone(),
                as_of: date,
                open_interest: Quantity::new("1250000".parse::<Decimal>().unwrap()),
            },
            OpenInterestSnapshot {
                instrument: other.clone(),
                as_of: date,
                open_interest: Quantity::new("875000.25".parse::<Decimal>().unwrap()),
            },
        ],
    )
    .unwrap();

    let store = ParquetOpenInterestStore::open(&dir.0).unwrap();
    let inst_latest = store.latest(&inst, None).await.unwrap().unwrap();
    assert_eq!(
        inst_latest.open_interest.0,
        Decimal::from(1_250_000),
        "exact decimal survives the Parquet round trip (scale may differ, value must not)"
    );
    let other_latest = store.latest(&other, None).await.unwrap().unwrap();
    assert_eq!(
        other_latest.open_interest.0,
        "875000.25".parse::<Decimal>().unwrap()
    );
}

// ——————————————————————————————————————————————————————————————————————————
// T3.2 — buildup classification
// ——————————————————————————————————————————————————————————————————————————

#[test]
fn classify_covers_all_four_quadrants() {
    assert_eq!(
        classify_buildup(Decimal::from(5), Decimal::from(100)),
        BuildupClass::LongBuildup
    );
    assert_eq!(
        classify_buildup(Decimal::from(-5), Decimal::from(100)),
        BuildupClass::ShortBuildup
    );
    assert_eq!(
        classify_buildup(Decimal::from(5), Decimal::from(-100)),
        BuildupClass::ShortCovering
    );
    assert_eq!(
        classify_buildup(Decimal::from(-5), Decimal::from(-100)),
        BuildupClass::LongUnwinding
    );
    // Zero deltas count as up (documented rule).
    assert_eq!(
        classify_buildup(Decimal::ZERO, Decimal::from(100)),
        BuildupClass::LongBuildup
    );
    assert_eq!(
        classify_buildup(Decimal::from(-5), Decimal::ZERO),
        BuildupClass::ShortBuildup
    );
}

/// Canned daily-bar source for the analyzer test.
#[derive(Default)]
struct FakeBarReader {
    bars: Vec<Bar>,
}

#[async_trait]
impl BarReader for FakeBarReader {
    async fn read_bars(&self, _request: &BarRequest) -> Result<Vec<Bar>, PortError> {
        Ok(self.bars.clone())
    }
}

fn daily_bar(symbol: &str, date: NaiveDate, close: f64) -> Bar {
    let bar_type = BarType::new(
        InstrumentId::new(Exchange::Nfo, symbol),
        tantra_market_store::wire::BarSpecification::new(
            1,
            tantra_market_store::wire::BarAggregation::Day,
            tantra_market_store::wire::PriceType::Last,
        ),
    );
    Bar::new(
        bar_type,
        close,
        close,
        close,
        close,
        0.0,
        UnixNanos::from_datetime(date.and_hms_opt(15, 30, 0).unwrap().and_utc()),
        UnixNanos::from_datetime(date.and_hms_opt(15, 30, 0).unwrap().and_utc()),
    )
}

#[tokio::test]
async fn analyzer_classifies_price_up_oi_up_as_long_buildup() {
    let oi = Arc::new(InMemoryOpenInterestStore::new());
    let d = |day| NaiveDate::from_ymd_opt(2026, 1, day).unwrap();
    let inst = contract("NIFTY26JANFUT");
    oi.put(OpenInterestSnapshot {
        instrument: inst.clone(),
        as_of: d(1),
        open_interest: Quantity::new(Decimal::from(1000)),
    })
    .await
    .unwrap();
    oi.put(OpenInterestSnapshot {
        instrument: inst.clone(),
        as_of: d(5),
        open_interest: Quantity::new(Decimal::from(1200)),
    })
    .await
    .unwrap();
    let bars = Arc::new(FakeBarReader {
        bars: vec![
            daily_bar("NIFTY26JANFUT", d(1), 100.0),
            daily_bar("NIFTY26JANFUT", d(5), 110.0),
        ],
    });
    let analyzer = BuildupAnalyzer::new(oi, bars);

    let signal = analyzer.buildup(&inst, d(1), d(5)).await.unwrap().unwrap();
    assert_eq!(signal.class, BuildupClass::LongBuildup);
    assert_eq!(signal.price_delta, Decimal::from(10));
    assert_eq!(signal.oi_delta, Decimal::from(200));
}

#[tokio::test]
async fn analyzer_returns_none_when_window_has_too_few_points() {
    let oi = Arc::new(InMemoryOpenInterestStore::new());
    let d = |day| NaiveDate::from_ymd_opt(2026, 1, day).unwrap();
    let inst = contract("NIFTY26JANFUT");
    oi.put(OpenInterestSnapshot {
        instrument: inst.clone(),
        as_of: d(1),
        open_interest: Quantity::new(Decimal::from(1000)),
    })
    .await
    .unwrap();
    let bars = Arc::new(FakeBarReader::default());
    let analyzer = BuildupAnalyzer::new(oi, bars);
    assert!(
        analyzer.buildup(&inst, d(1), d(5)).await.unwrap().is_none(),
        "a single observation cannot yield a delta"
    );
}
