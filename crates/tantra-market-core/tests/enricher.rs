//! Market data enricher port tests. The enricher is the pure (WASM-portable)
//! microstructure component of the source `MarketDataEnricher`.

use chrono::Utc;
use rust_decimal::Decimal;
use tantra_core::{Exchange, InstrumentId, Price, PriceBand, Quantity, Tick};
use tantra_market_core::MarketDataEnricher;

#[test]
fn enricher_attaches_price_band_to_quote_ticks() {
    let inst = InstrumentId::new(Exchange::Nse, "INFY");
    let mut enricher = MarketDataEnricher::new();
    let band = PriceBand::new(
        Price::from_rupees("1500.00").unwrap(),
        Price::from_rupees("1650.00").unwrap(),
        Price::from_rupees("1575.00").unwrap(),
    );
    enricher.set_price_band(inst.clone(), band);

    let now = Utc::now();
    let mut tick = Tick::Quote {
        instrument: inst.clone(),
        bid: Price::from_rupees("1600.00").unwrap(),
        ask: Price::from_rupees("1600.50").unwrap(),
        bid_qty: Quantity::from_u64(100),
        ask_qty: Quantity::from_u64(150),
        last_price: Some(Price::from_rupees("1600.25").unwrap()),
        exchange_ts: now,
        receive_ts: now,
        band: None,
    };

    enricher.enrich(&mut tick);
    match &tick {
        Tick::Quote { band: b, .. } => assert_eq!(*b, Some(band)),
        _ => panic!("expected quote tick"),
    }
}

#[test]
fn enricher_tracks_intraday_vwap_from_ltp_ticks() {
    let inst = InstrumentId::new(Exchange::Nse, "WIPRO");
    let mut enricher = MarketDataEnricher::new();
    let now = Utc::now();

    let mut tick_a = Tick::Ltp {
        instrument: inst.clone(),
        price: Price::from_rupees("400.00").unwrap(),
        volume: Quantity::from_u64(100),
        exchange_ts: now,
        receive_ts: now,
    };
    let mut tick_b = Tick::Ltp {
        instrument: inst.clone(),
        price: Price::from_rupees("402.00").unwrap(),
        volume: Quantity::from_u64(100),
        exchange_ts: now,
        receive_ts: now,
    };
    enricher.enrich(&mut tick_a);
    enricher.enrich(&mut tick_b);

    // (400.00 * 100 + 402.00 * 100) / 200 = 401.00
    let vwap = enricher.vwap(&inst).expect("vwap after ltp ticks");
    assert_eq!(vwap.value(), Decimal::from(401));
}
