mod common;
use common::hlc;
use rust_decimal::Decimal;
use tantra_market_indicators::screener::{
    FilterOp, MetricRef, PredicateValue, ScreenerError, ScreenerFilterPredicate, evaluate_predicate,
};

#[test]
fn test_predicate_gt() {
    let bars = vec![hlc(10.0, 5.0, 8.0)];
    let pred = ScreenerFilterPredicate::new(
        MetricRef::new("close"),
        FilterOp::Gt,
        PredicateValue::Decimal(Decimal::from(7)),
    );
    assert!(evaluate_predicate(&pred, &bars).unwrap());
}

#[test]
fn test_unsupported_metric() {
    let bars = vec![hlc(10.0, 5.0, 8.0)];
    let pred = ScreenerFilterPredicate::new(
        MetricRef::new("unknown"),
        FilterOp::Eq,
        PredicateValue::Decimal(Decimal::from(10)),
    );
    let err = evaluate_predicate(&pred, &bars).unwrap_err();
    assert!(matches!(err, ScreenerError::UnsupportedMetric { .. }));
}
