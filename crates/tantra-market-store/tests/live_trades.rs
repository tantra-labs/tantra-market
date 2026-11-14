//! Integration tests for the live trade snapshot store: [`TradeReader`] on
//! [`LiveQuoteStore`] (runtime gap `tantra-runtime/docs/gaps/0007` items 1-2).

use std::sync::Arc;

use tantra_core::{Exchange, InstrumentId};
use tantra_market_store::ports::{TradeReader, TradeSnapshot};
use tantra_market_store::wire::{AggressorSide, TradeId, TradeTick, UnixNanos};
use tantra_market_store::{Dataset, DatasetReader, LiveQuoteStore};

fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::new(Exchange::Nse, symbol)
}

fn utc_ts(date: (i32, u32, u32), hms: (u32, u32, u32)) -> UnixNanos {
    UnixNanos::from_datetime(
        chrono::NaiveDate::from_ymd_opt(date.0, date.1, date.2)
            .unwrap()
            .and_hms_opt(hms.0, hms.1, hms.2)
            .unwrap()
            .and_utc(),
    )
}

fn trade(price: f64, size: f64, ts_event: UnixNanos) -> TradeTick {
    TradeTick::new(
        instrument("RELIANCE-EQ"),
        price,
        size,
        AggressorSide::Buyer,
        TradeId::new("t1"),
        ts_event,
        ts_event,
    )
}

fn store() -> LiveQuoteStore {
    LiveQuoteStore::new(Arc::new(DatasetReader::from_dataset(Dataset::default())))
}

#[tokio::test]
async fn trade_reader_returns_none_until_a_trade_is_put() {
    let store = store();
    let id = instrument("RELIANCE-EQ");
    assert_eq!(store.read_trade_snapshot(&id).await.unwrap(), None);
}

#[tokio::test]
async fn put_trade_tracks_latest_print_monotonically() {
    let store = store();
    let id = instrument("RELIANCE-EQ");
    let earlier = utc_ts((2026, 1, 5), (9, 0, 0));
    let later = utc_ts((2026, 1, 5), (9, 0, 1));

    store.put_trade(trade(100.0, 10.0, earlier));
    store.put_trade(trade(105.0, 5.0, later));

    let TradeSnapshot {
        last, day_volume, ..
    } = store
        .read_trade_snapshot(&id)
        .await
        .unwrap()
        .expect("a trade was put");
    assert_eq!(last.price(), 105.0, "the later print wins");
    assert_eq!(day_volume, 15.0);

    // A stale print must neither regress the latest price nor the day volume.
    let stale = utc_ts((2026, 1, 5), (8, 0, 0));
    store.put_trade(trade(90.0, 100.0, stale));

    let snapshot = store
        .read_trade_snapshot(&id)
        .await
        .unwrap()
        .expect("a trade was put");
    assert_eq!(snapshot.last.price(), 105.0);
    assert_eq!(snapshot.day_volume, 15.0);
}

#[tokio::test]
async fn put_trade_accumulates_session_day_volume() {
    let store = store();
    let id = instrument("RELIANCE-EQ");

    store.put_trade(trade(100.0, 10.0, utc_ts((2026, 1, 5), (9, 0, 0))));
    store.put_trade(trade(101.0, 5.0, utc_ts((2026, 1, 5), (9, 5, 0))));
    let snapshot = store
        .read_trade_snapshot(&id)
        .await
        .unwrap()
        .expect("a trade was put");
    assert_eq!(
        snapshot.day_volume, 15.0,
        "same-day sizes accumulate to 15.0"
    );

    // A trade on the next UTC day resets the accumulator.
    store.put_trade(trade(102.0, 7.0, utc_ts((2026, 1, 6), (9, 0, 0))));
    let snapshot = store
        .read_trade_snapshot(&id)
        .await
        .unwrap()
        .expect("a trade was put");
    assert_eq!(snapshot.day_volume, 7.0, "a new day resets the volume");
    assert_eq!(snapshot.last.price(), 102.0);
}
