//! Unit tests for `vyapaar_market_finance::universes`.

use chrono::NaiveDate;

use vyapaar_market_finance::{StaticUniverse, Universe};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[test]
fn static_universe_answers_membership_and_position() {
    let u = StaticUniverse::new(date(2025, 1, 1), vec!["A".into(), "B".into()]);
    assert_eq!(u.as_of(), date(2025, 1, 1));
    assert_eq!(u.len(), 2);
    assert!(!u.is_empty());
    assert!(u.contains("B"));
    assert!(!u.contains("b"));
    assert_eq!(u.position("B"), Some(1));
    assert_eq!(u.position("Z"), None);
}

#[test]
fn empty_universe_is_empty() {
    let u = StaticUniverse::new(date(2025, 1, 1), vec![]);
    assert!(u.is_empty());
    assert_eq!(u.len(), 0);
}
