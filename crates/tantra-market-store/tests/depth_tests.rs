//! Integration tests for the L2 order-book depth stores: [`LiveQuoteStore::put_depth`] and
//! the parquet depth ticks of [`ParquetTickStore`] (closes `tantra-runtime/docs/gaps/0007`
//! item 10 / `tantra-market/docs/gaps/0006` — the exact-Decimal `DepthReader` port).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tantra_core::{Exchange, InstrumentId, Price, Quantity};
use tantra_market_store::ports::{DepthReader, DepthSnapshot};
use tantra_market_store::wire::UnixNanos;
use tantra_market_store::{
    Dataset, DatasetReader, DepthTick, LiveQuoteStore, ParquetTickStore, TICK_DIR, write_depth_file,
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tantra_depth_it_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::new(Exchange::Nse, symbol)
}

fn price(value: &str) -> Price {
    Price::new(value.parse().unwrap())
}

fn qty(value: &str) -> Quantity {
    Quantity::new(value.parse().unwrap())
}

fn level(price_raw: &str, qty_raw: &str) -> (Price, Quantity) {
    (price(price_raw), qty(qty_raw))
}

fn snapshot(ts: u64) -> DepthSnapshot {
    DepthSnapshot {
        as_of: UnixNanos::from_u64(ts),
        bids: vec![level("100.50", "100"), level("100.25", "250")],
        asks: vec![level("100.75", "150"), level("101.00", "50")],
    }
}

fn live_store() -> LiveQuoteStore {
    LiveQuoteStore::new(Arc::new(DatasetReader::from_dataset(Dataset::default())))
}

#[tokio::test]
async fn live_store_answers_unsupported_until_depth_is_put() {
    let store = live_store();
    let id = instrument("RELIANCE-EQ");
    let err = store.read_depth(&id, 5).await.unwrap_err();
    assert!(matches!(
        err,
        tantra_market_store::ports::PortError::Unsupported(_)
    ));
}

#[tokio::test]
async fn live_store_put_depth_serves_the_snapshot_truncated_to_levels() {
    let store = live_store();
    let id = instrument("RELIANCE-EQ");
    store.put_depth(id.clone(), snapshot(1_700_000_000_000_000_000));

    let book = store.read_depth(&id, 1).await.unwrap();
    assert_eq!(book.as_of.as_u64(), 1_700_000_000_000_000_000);
    assert_eq!(book.bids.len(), 1);
    assert_eq!(book.asks.len(), 1);
    assert_eq!(book.bids[0], level("100.50", "100"));

    let full = store.read_depth(&id, 10).await.unwrap();
    assert_eq!(full.bids.len(), 2);
    assert_eq!(full.asks.len(), 2);
}

#[tokio::test]
async fn parquet_store_roundtrips_depth_ticks() {
    let tmp = TempDir::new("parquet_roundtrip");
    let path = tmp.as_ref().join(TICK_DIR).join("depth.parquet");
    write_depth_file(
        &path,
        &[DepthTick {
            instrument: instrument("RELIANCE-EQ"),
            ts_event: UnixNanos::from_u64(1_700_000_000_000_000_000),
            snapshot: snapshot(1_700_000_000_000_000_000),
        }],
    )
    .unwrap();

    let store = ParquetTickStore::open(tmp.as_ref()).unwrap();
    let (quotes, trades, depth) = store.len();
    assert_eq!((quotes, trades), (0, 0));
    assert_eq!(depth, 1);

    let book = store
        .read_depth(&instrument("RELIANCE-EQ"), 5)
        .await
        .unwrap();
    assert_eq!(book.as_of.as_u64(), 1_700_000_000_000_000_000);
    assert_eq!(
        book.bids,
        vec![level("100.50", "100"), level("100.25", "250")]
    );
    assert_eq!(
        book.asks,
        vec![level("100.75", "150"), level("101.00", "50")]
    );
    assert_eq!(
        book.bids[0].0.value().to_string(),
        "100.50",
        "exact decimal survives the parquet round trip"
    );
}

#[tokio::test]
async fn parquet_store_answers_unsupported_for_instruments_without_depth() {
    let tmp = TempDir::new("parquet_no_depth");
    let path = tmp.as_ref().join(TICK_DIR).join("depth.parquet");
    write_depth_file(
        &path,
        &[DepthTick {
            instrument: instrument("RELIANCE-EQ"),
            ts_event: UnixNanos::from_u64(1),
            snapshot: snapshot(1),
        }],
    )
    .unwrap();
    let store = ParquetTickStore::open(tmp.as_ref()).unwrap();

    let err = store.read_depth(&instrument("TCS"), 5).await.unwrap_err();
    assert!(matches!(
        err,
        tantra_market_store::ports::PortError::Unsupported(_)
    ));
}

#[tokio::test]
async fn parquet_store_latest_depth_wins() {
    let tmp = TempDir::new("parquet_latest");
    let path = tmp.as_ref().join(TICK_DIR).join("depth.parquet");
    let mut early = snapshot(1);
    early.as_of = UnixNanos::from_u64(1);
    let mut late = snapshot(2);
    late.as_of = UnixNanos::from_u64(2);
    late.bids[0] = level("105.00", "900");
    write_depth_file(
        &path,
        &[
            DepthTick {
                instrument: instrument("RELIANCE-EQ"),
                ts_event: UnixNanos::from_u64(1),
                snapshot: early,
            },
            DepthTick {
                instrument: instrument("RELIANCE-EQ"),
                ts_event: UnixNanos::from_u64(2),
                snapshot: late,
            },
        ],
    )
    .unwrap();
    let store = ParquetTickStore::open(tmp.as_ref()).unwrap();
    let book = store
        .read_depth(&instrument("RELIANCE-EQ"), 5)
        .await
        .unwrap();
    assert_eq!(book.bids[0], level("105.00", "900"), "the latest tick wins");
}
