//! Shared fixtures for the extracted in-crate tests (ported from
//! `honba-me-market/src/tests/mod.rs`).

use chrono::NaiveDate;

use tantra_market_finance::{MarketCalendar, Session};

pub fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// A weekday calendar with an explicit holiday list.
pub struct Holidays(pub Vec<NaiveDate>);

impl MarketCalendar for Holidays {
    fn session(&self) -> Session {
        Session::regular()
    }

    fn is_holiday(&self, date: NaiveDate) -> bool {
        self.0.contains(&date)
    }
}
