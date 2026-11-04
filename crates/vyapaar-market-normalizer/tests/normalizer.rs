//! Unit tests for the instrument master and corporate actions engine.
//!
//! The upstream source crate had no test suite; these tests lock in the
//! byte-slice loader contract introduced by the WASM-purification refactor
//! (no filesystem access inside this crate).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use vyapaar_core::{Exchange, Price};
use vyapaar_market_normalizer::{
    CorporateActionEvent, CorporateActionType, CorporateActions, InstrumentMaster,
    InstrumentMasterEntry,
};

const ZERODHA_CSV: &str = concat!(
    "instrument_token,exchange_token,tradingsymbol,name,last_price,expiry,strike,tick_size,lot_size,instrument_type,segment,exchange\n",
    "408065,1234,INFY,Infosys,1600.00,,,0.05,1,EQ,NSE,NSE\n",
    "999001,777,,Blank Symbol,,,,,0.05,EQ,NSE,NSE\n",
    "11536,604,TCS,Tata Consultancy,4000.00,,,0.05,1,EQ,BSE,BSE\n"
);

const NSE_EQUITY_CSV: &str = concat!(
    "SYMBOL,SERIES,DATE_OF_LISTING,PAID_UP_VALUE,MARKET_LOT,ISIN_NUMBER,FACE_VALUE\n",
    "RELIANCE,EQ,01-Jul-1980,10,1,INE002A01018,10.0\n",
    "TESTDEL,BE,01-Jan-2026,10,1,INE999A01019,1.0\n"
);

const UPSTOX_JSON: &str = r#"[
  {
    "instrument_token": "2048065",
    "exchange_token": "1234",
    "trading_symbol": "ONGC",
    "name": "Oil & Natural Gas",
    "expiry": null,
    "strike": null,
    "tick_size": "0.05",
    "lot_size": "75",
    "instrument_type": "EQUITY",
    "segment": "EQ",
    "exchange": "NSE"
  }
]"#;

fn d(s: &str) -> Decimal {
    s.parse().expect("exact decimal")
}

#[test]
fn zerodha_csv_payload_populates_token_symbol_and_lot_lookup() {
    let mut master = InstrumentMaster::new();
    let count = master.load_zerodha_csv(ZERODHA_CSV.as_bytes()).unwrap();
    // The row with an empty tradingsymbol is skipped.
    assert_eq!(count, 2);
    assert_eq!(master.len(), 2);

    let infy = master.by_token("408065").expect("token index").clone();
    assert_eq!(infy.symbol, "INFY");
    assert_eq!(infy.exchange, Exchange::Nse);

    assert_eq!(master.by_symbol("INFY"), Some(&infy));

    let entry = master.entry(&infy).expect("entry");
    assert_eq!(entry.lot_size, Some(1));
    assert_eq!(
        entry.tick_size.map(|p| p.value()),
        Some(Decimal::from_str_exact("0.05").unwrap())
    );

    let tcs = master.by_symbol("TCS").expect("symbol index");
    assert_eq!(master.resolve_tick_size(tcs).unwrap().value(), d("0.05"));
    assert_eq!(master.resolve_lot_size(tcs), Some(1));
    assert!(master.resolve_lot_size(&infy).is_some());
}

#[test]
fn upstox_json_payload_populates_lot_size_and_symbol() {
    let mut master = InstrumentMaster::new();
    let count = master.load_upstox_json(UPSTOX_JSON.as_bytes()).unwrap();
    assert_eq!(count, 1);

    let ongc = master.by_token("2048065").expect("token").clone();
    assert_eq!(ongc.symbol, "ONGC");
    let entry = master.entry(&ongc).unwrap();
    assert_eq!(entry.lot_size, Some(75));
    assert_eq!(master.resolve_tick_size(&ongc).unwrap().value(), d("0.05"));
}

#[test]
fn nse_equity_csv_payload_maps_isin_and_series() {
    let mut master = InstrumentMaster::new();
    let count = master
        .load_nse_equity_csv(NSE_EQUITY_CSV.as_bytes())
        .unwrap();
    assert_eq!(count, 2, "both non-empty SYMBOL rows are imported");

    let reliance = master.by_isin("INE002A01018").expect("ISIN index").clone();
    assert_eq!(reliance.symbol, "RELIANCE");

    let be_entry = master.by_isin("INE999A01019").expect("BE ISIN").clone();
    let entry: &InstrumentMasterEntry = master.entry(&be_entry).unwrap();
    assert_eq!(entry.instrument_type.as_deref(), Some("BE"));
}

#[test]
fn master_export_import_round_trips_every_entry() {
    let mut master = InstrumentMaster::new();
    master.load_zerodha_csv(ZERODHA_CSV.as_bytes()).unwrap();

    let exported = master.export_master();
    assert_eq!(exported.len(), 2);

    let mut restored = InstrumentMaster::new();
    restored.import_master(&exported);
    assert_eq!(restored.len(), 2);
    assert_eq!(
        restored.by_token("11536").map(|i| i.symbol.clone()),
        Some("TCS".to_string())
    );
    assert_eq!(restored.export_master().len(), master.export_master().len());
}

#[test]
fn corporate_actions_split_and_bonus_compose_multiplicative_adjustment() {
    let date_of_split = NaiveDate::from_ymd_opt(2025, 6, 2).unwrap();
    let date_of_bonus = NaiveDate::from_ymd_opt(2024, 9, 1).unwrap();

    let mut actions = CorporateActions::new();
    actions.add_event(CorporateActionEvent {
        isin: "INE002A01018".into(),
        event_type: CorporateActionType::Split {
            old_shares: 1,
            new_shares: 2,
        },
        effective_date: date_of_split,
        announcement_date: None,
        record_date: None,
    });
    actions.add_event(CorporateActionEvent {
        isin: "INE002A01018".into(),
        event_type: CorporateActionType::Bonus {
            bonus_ratio_num: 1,
            bonus_ratio_den: 3,
        },
        effective_date: date_of_bonus,
        announcement_date: None,
        record_date: None,
    });

    // Factor formula: den / (num + den) = 3 / 4.
    let bonus_only = actions.adjustment_factor("INE002A01018", date_of_bonus);
    assert_eq!(bonus_only, d("0.75"));

    // Bonus (0.75) * Split (1/2 = 0.5) = 0.375.
    let both = actions.adjustment_factor(
        "INE002A01018",
        NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(),
    );
    assert_eq!(both, d("0.375"));

    // Far before every event: factor 1.
    assert_eq!(
        actions.adjustment_factor("INE002A01018", NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()),
        Decimal::ONE
    );
}

#[test]
fn corporate_actions_adjust_price_scales_the_historical_price() {
    let mut actions = CorporateActions::new();
    actions.add_event(CorporateActionEvent {
        isin: "INE123A01018".into(),
        event_type: CorporateActionType::Split {
            old_shares: 1,
            new_shares: 10,
        },
        effective_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
        announcement_date: None,
        record_date: None,
    });

    let historical = Price::try_new(Decimal::from(1_000)).unwrap();
    let adjusted = actions.adjust_price(
        "INE123A01018",
        historical,
        NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
    );
    assert_eq!(adjusted.value(), Decimal::from(100));
}
