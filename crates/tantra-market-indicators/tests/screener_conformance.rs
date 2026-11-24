//! Integration tests for the pure screener evaluator against canonical `tantra-core` types.

use std::str::FromStr;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use tantra_core::{Bar, Exchange, InstrumentId, Price, Quantity};
use tantra_market_indicators::screener::{
    FilterOp, MetricKeySpec, MetricPeriod, MetricRef, PredicateValue, ScreenerError,
    ScreenerFilterGroup, ScreenerFilterItem, ScreenerFilterPredicate, check_metric, evaluate_group,
    evaluate_predicate, latest_metrics,
};

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn make_bar(open: &str, high: &str, low: &str, close: &str, volume: u64, day_idx: i64) -> Bar {
    Bar {
        instrument: InstrumentId::try_new(Exchange::Nse, "X").unwrap(),
        open: Price::try_new(dec(open)).unwrap(),
        high: Price::try_new(dec(high)).unwrap(),
        low: Price::try_new(dec(low)).unwrap(),
        close: Price::try_new(dec(close)).unwrap(),
        volume: Quantity::from_u64(volume),
        timestamp: Utc.timestamp_opt(day_idx * 86_400, 0).unwrap(),
    }
}

fn pred(key: &str, op: FilterOp, value: &str) -> ScreenerFilterPredicate {
    ScreenerFilterPredicate::new(MetricRef::new(key), op, PredicateValue::Decimal(dec(value)))
}

#[test]
fn predicate_comparisons_work_on_bars() {
    let data = vec![
        make_bar("10.0", "11.0", "9.5", "10.25", 100, 0),
        make_bar("10.25", "11.5", "10.0", "10.50", 150, 1),
        make_bar("10.50", "11.25", "10.25", "10.75", 200, 2),
    ];

    assert_eq!(
        evaluate_predicate(&pred("close", FilterOp::Gt, "10.0"), &data),
        Ok(true)
    );
    assert_eq!(
        evaluate_predicate(&pred("close", FilterOp::Gt, "10.75"), &data),
        Ok(false)
    );
    assert_eq!(
        evaluate_predicate(&pred("close", FilterOp::Gte, "10.75"), &data),
        Ok(true)
    );
    assert_eq!(
        evaluate_predicate(&pred("close", FilterOp::Lt, "11.0"), &data),
        Ok(true)
    );
    assert_eq!(
        evaluate_predicate(&pred("close", FilterOp::Lte, "10.75"), &data),
        Ok(true)
    );
    assert_eq!(
        evaluate_predicate(&pred("close", FilterOp::Eq, "10.75"), &data),
        Ok(true)
    );
    assert_eq!(
        evaluate_predicate(&pred("close", FilterOp::Ne, "10.0"), &data),
        Ok(true)
    );
    assert_eq!(
        evaluate_predicate(&pred("close", FilterOp::CrossesAbove, "10.60"), &data),
        Ok(true)
    );
    assert_eq!(
        evaluate_predicate(
            &ScreenerFilterPredicate::between(
                MetricRef::new("close"),
                PredicateValue::Decimal(dec("10.50")),
                PredicateValue::Decimal(dec("11.00")),
            ),
            &data
        ),
        Ok(true)
    );

    let metrics = latest_metrics(&["close".to_string(), "sma3".to_string()], &data).unwrap();
    assert_eq!(metrics["close"], Some(dec("10.75")));
    assert_eq!(metrics["sma3"], Some(dec("10.5")));
}

#[test]
fn group_evaluation_and_or() {
    let data = vec![
        make_bar("10.0", "11.0", "9.5", "10.25", 100, 0),
        make_bar("10.25", "11.5", "10.0", "10.50", 150, 1),
        make_bar("10.50", "11.25", "10.25", "10.75", 200, 2),
    ];

    let group_and = ScreenerFilterGroup::and(vec![
        ScreenerFilterItem::Predicate(pred("close", FilterOp::Gt, "10.0")),
        ScreenerFilterItem::Predicate(pred("volume", FilterOp::Gte, "200")),
    ]);
    assert_eq!(evaluate_group(&group_and, &data), Ok(true));

    let group_or = ScreenerFilterGroup::or(vec![
        ScreenerFilterItem::Predicate(pred("close", FilterOp::Lt, "5.0")),
        ScreenerFilterItem::Predicate(pred("volume", FilterOp::Eq, "200")),
    ]);
    assert_eq!(evaluate_group(&group_or, &data), Ok(true));

    let nested = ScreenerFilterGroup::and(vec![
        ScreenerFilterItem::Predicate(pred("close", FilterOp::Gt, "10.0")),
        ScreenerFilterItem::Group(Box::new(group_or)),
    ]);
    assert_eq!(evaluate_group(&nested, &data), Ok(true));
}

#[test]
fn unsupported_metrics_and_periods_are_rejected() {
    let data = vec![make_bar("10.0", "11.0", "9.5", "10.25", 100, 0)];
    assert!(check_metric("close").is_ok());
    assert!(matches!(
        check_metric("pe_ratio"),
        Err(ScreenerError::UnsupportedMetric { .. })
    ));

    let bad_period = ScreenerFilterPredicate::new(
        MetricRef(MetricKeySpec::new("close").with_period(MetricPeriod::Ttm)),
        FilterOp::Gt,
        PredicateValue::Decimal(dec("5.0")),
    );
    assert!(matches!(
        evaluate_predicate(&bad_period, &data),
        Err(ScreenerError::UnsupportedMetric { .. })
    ));
}
