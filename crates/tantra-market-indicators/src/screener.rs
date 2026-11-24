//! Screener predicate evaluation over a bar series.
//!
//! Converged onto the canonical [`tantra_core::screener`] wire AST with pure
//! `rust_decimal::Decimal` arithmetic (zero floats).

use std::collections::BTreeMap;
use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tantra_core::Bar;
pub use tantra_core::screener::{
    FilterOp, LogicalOp, MetricKeySpec, MetricPeriod, MetricRef, PredicateValue,
    ScreenerFilterGroup, ScreenerFilterItem, ScreenerFilterPredicate,
};
pub use tantra_core::signal::Timeframe;

/// Screener evaluation error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScreenerError {
    /// The requested metric is not supported.
    UnsupportedMetric { key: String },
    /// The operand type is invalid for the operator.
    InvalidOperand { op: FilterOp, message: String },
    /// The filter definition is invalid.
    InvalidFilter { message: String },
}

impl fmt::Display for ScreenerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedMetric { key } => write!(f, "unsupported metric: {key}"),
            Self::InvalidOperand { op, message } => {
                write!(f, "invalid operand for {op:?}: {message}")
            }
            Self::InvalidFilter { message } => write!(f, "invalid filter: {message}"),
        }
    }
}

impl std::error::Error for ScreenerError {}

/// Bars in the 52-week window; fewer bars than this is insufficient data.
const WEEK52_BARS: usize = 252;

/// A bar-derived metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Open,
    High,
    Low,
    Close,
    Volume,
    Week52Low,
    Week52High,
    Sma(usize),
    Rsi14,
}

fn parse_metric_inner(key: &str) -> Result<Metric, ScreenerError> {
    let k = key.to_ascii_lowercase();
    match k.as_str() {
        "open" => Ok(Metric::Open),
        "high" => Ok(Metric::High),
        "low" => Ok(Metric::Low),
        "close" => Ok(Metric::Close),
        "volume" => Ok(Metric::Volume),
        "price_52_week_low" => Ok(Metric::Week52Low),
        "price_52_week_high" => Ok(Metric::Week52High),
        s if s.starts_with("sma") => {
            let period: usize = s[3..]
                .parse()
                .map_err(|_| ScreenerError::UnsupportedMetric {
                    key: key.to_owned(),
                })?;
            if period == 0 {
                return Err(ScreenerError::UnsupportedMetric {
                    key: key.to_owned(),
                });
            }
            Ok(Metric::Sma(period))
        }
        "rsi14" | "rsi" => Ok(Metric::Rsi14),
        _ => Err(ScreenerError::UnsupportedMetric {
            key: key.to_owned(),
        }),
    }
}

/// Parses a metric key string into a [`Metric`].
pub fn parse_metric(key: &str) -> Result<Metric, ScreenerError> {
    parse_metric_inner(key)
}

/// Checks that `key` is a metric this evaluator can compute from bars.
pub fn check_metric(key: &str) -> Result<(), ScreenerError> {
    parse_metric(key).map(|_| ())
}

fn series(metric: Metric, bars: &[Bar]) -> Vec<Option<Decimal>> {
    match metric {
        Metric::Open => bars.iter().map(|b| Some(b.open.value())).collect(),
        Metric::High => bars.iter().map(|b| Some(b.high.value())).collect(),
        Metric::Low => bars.iter().map(|b| Some(b.low.value())).collect(),
        Metric::Close => bars.iter().map(|b| Some(b.close.value())).collect(),
        Metric::Volume => bars.iter().map(|b| Some(b.volume.value())).collect(),
        Metric::Week52Low => (0..bars.len())
            .map(|i| {
                let start = (i + 1usize).saturating_sub(WEEK52_BARS);
                bars[start..=i].iter().map(|b| b.low.value()).min()
            })
            .collect(),
        Metric::Week52High => (0..bars.len())
            .map(|i| {
                let start = (i + 1usize).saturating_sub(WEEK52_BARS);
                bars[start..=i].iter().map(|b| b.high.value()).max()
            })
            .collect(),
        Metric::Sma(period) => {
            let denom = Decimal::from(period);
            (0..bars.len())
                .map(|i| {
                    if i + 1 < period {
                        None
                    } else {
                        let sum: Decimal = bars[i + 1 - period..=i]
                            .iter()
                            .map(|b| b.close.value())
                            .sum();
                        Some(sum / denom)
                    }
                })
                .collect()
        }
        Metric::Rsi14 => {
            const PERIOD: usize = 14;
            let hundred = Decimal::from(100);
            (0..bars.len())
                .map(|i| {
                    if i < PERIOD {
                        return None;
                    }
                    let window = &bars[i - PERIOD..=i];
                    let mut gains = Decimal::ZERO;
                    let mut losses = Decimal::ZERO;
                    for j in 1..=PERIOD {
                        let diff = window[j].close.value() - window[j - 1].close.value();
                        if diff > Decimal::ZERO {
                            gains += diff;
                        } else {
                            losses -= diff;
                        }
                    }
                    if losses == Decimal::ZERO {
                        Some(hundred)
                    } else if gains == Decimal::ZERO {
                        Some(Decimal::ZERO)
                    } else {
                        let rs = gains / losses;
                        Some(hundred - (hundred / (Decimal::ONE + rs)))
                    }
                })
                .collect()
        }
    }
}

/// The latest value of each metric in `keys` (`None` while it warms up), keyed as asked.
pub fn latest_metrics(
    keys: &[String],
    bars: &[Bar],
) -> Result<BTreeMap<String, Option<Decimal>>, ScreenerError> {
    let mut out = BTreeMap::new();
    for key in keys {
        let metric = parse_metric(key)?;
        out.insert(key.clone(), series(metric, bars).last().copied().flatten());
    }
    Ok(out)
}

fn check_dimensions(key: &str, period: Option<MetricPeriod>) -> Result<(), ScreenerError> {
    match period {
        None
        | Some(
            MetricPeriod::Snapshot
            | MetricPeriod::Current
            | MetricPeriod::Daily
            | MetricPeriod::Intraday
            | MetricPeriod::Weekly
            | MetricPeriod::Monthly,
        ) => Ok(()),
        Some(_) => Err(ScreenerError::UnsupportedMetric {
            key: key.to_owned(),
        }),
    }
}

fn resolve_scalar_or_metric_pair(
    op: FilterOp,
    val: &PredicateValue,
    bars: &[Bar],
) -> Result<Option<(Decimal, Option<Decimal>)>, ScreenerError> {
    match val {
        PredicateValue::Decimal(d) => Ok(Some((*d, Some(*d)))),
        PredicateValue::MetricRef(m) => {
            check_dimensions(&m.0.key, m.0.period)?;
            let metric = parse_metric(&m.0.key)?;
            if matches!(metric, Metric::Week52Low | Metric::Week52High) && bars.len() < WEEK52_BARS
            {
                return Ok(None);
            }
            let s = series(metric, bars);
            let Some(curr) = s.last().copied().flatten() else {
                return Ok(None);
            };
            let prev = s.len().checked_sub(2).and_then(|idx| s[idx]);
            Ok(Some((curr, prev)))
        }
        other => Err(ScreenerError::InvalidOperand {
            op,
            message: format!("expected Decimal or MetricRef, got {other:?}"),
        }),
    }
}

/// Evaluates one predicate against a chronological bar history (latest bar last).
pub fn evaluate_predicate(
    pred: &ScreenerFilterPredicate,
    bars: &[Bar],
) -> Result<bool, ScreenerError> {
    if pred.relative_to.is_some() || pred.relative_stat.is_some() {
        return Err(ScreenerError::InvalidFilter {
            message: "relative_to / relative_stat requires cross-sectional universe context"
                .to_owned(),
        });
    }
    pred.validate().map_err(|e| ScreenerError::InvalidFilter {
        message: e.to_string(),
    })?;

    let lhs_metric = parse_metric(&pred.metric.0.key)?;
    check_dimensions(&pred.metric.0.key, pred.metric.0.period)?;
    if bars.is_empty() {
        return Ok(false);
    }
    if matches!(lhs_metric, Metric::Week52Low | Metric::Week52High) && bars.len() < WEEK52_BARS {
        return Ok(false);
    }
    let lhs_series = series(lhs_metric, bars);
    let Some(lhs_curr) = lhs_series.last().copied().flatten() else {
        return Ok(false);
    };
    let lhs_prev = lhs_series.len().checked_sub(2).and_then(|i| lhs_series[i]);

    match pred.op {
        FilterOp::CrossesAbove | FilterOp::CrossesBelow => {
            let Some((rhs_curr, Some(rhs_prev))) =
                resolve_scalar_or_metric_pair(pred.op, &pred.value, bars)?
            else {
                return Ok(false);
            };
            let Some(lp) = lhs_prev else {
                return Ok(false);
            };
            Ok(if pred.op == FilterOp::CrossesAbove {
                lp <= rhs_prev && lhs_curr > rhs_curr
            } else {
                lp >= rhs_prev && lhs_curr < rhs_curr
            })
        }
        FilterOp::Between => {
            let Some((lo_v, hi_v)) = pred.between_bounds() else {
                return Err(ScreenerError::InvalidOperand {
                    op: pred.op,
                    message: "`between` requires a 2-element list".to_owned(),
                });
            };
            let Some((lo, _)) = resolve_scalar_or_metric_pair(pred.op, lo_v, bars)? else {
                return Ok(false);
            };
            let Some((hi, _)) = resolve_scalar_or_metric_pair(pred.op, hi_v, bars)? else {
                return Ok(false);
            };
            Ok(lo <= lhs_curr && lhs_curr <= hi)
        }
        FilterOp::In | FilterOp::NotIn => {
            let Some(items) = pred.value.as_list() else {
                return Err(ScreenerError::InvalidOperand {
                    op: pred.op,
                    message: format!("`{}` requires a list operand", pred.op),
                });
            };
            let mut found = false;
            for item in items {
                if let Some((v, _)) = resolve_scalar_or_metric_pair(pred.op, item, bars)?
                    && lhs_curr == v
                {
                    found = true;
                    break;
                }
            }
            Ok(if pred.op == FilterOp::In {
                found
            } else {
                !found
            })
        }
        FilterOp::Like | FilterOp::Has => {
            let needle = match &pred.value {
                PredicateValue::String(s) => s.to_lowercase(),
                PredicateValue::Decimal(d) => d.normalize().to_string().to_lowercase(),
                PredicateValue::Bool(b) => b.to_string().to_lowercase(),
                other => {
                    return Err(ScreenerError::InvalidOperand {
                        op: pred.op,
                        message: format!("expected scalar operand, got {other:?}"),
                    });
                }
            };
            Ok(lhs_curr
                .normalize()
                .to_string()
                .to_lowercase()
                .contains(&needle))
        }
        FilterOp::Eq
        | FilterOp::Ne
        | FilterOp::Gt
        | FilterOp::Gte
        | FilterOp::Lt
        | FilterOp::Lte => {
            let Some((rhs_curr, _)) = resolve_scalar_or_metric_pair(pred.op, &pred.value, bars)?
            else {
                return Ok(false);
            };
            Ok(match pred.op {
                FilterOp::Eq => lhs_curr == rhs_curr,
                FilterOp::Ne => lhs_curr != rhs_curr,
                FilterOp::Gt => lhs_curr > rhs_curr,
                FilterOp::Gte => lhs_curr >= rhs_curr,
                FilterOp::Lt => lhs_curr < rhs_curr,
                FilterOp::Lte => lhs_curr <= rhs_curr,
                _ => unreachable!(),
            })
        }
    }
}

/// Evaluates a recursive filter group against a bar history.
pub fn evaluate_group(group: &ScreenerFilterGroup, bars: &[Bar]) -> Result<bool, ScreenerError> {
    if group.classification.as_ref().is_some_and(|c| !c.is_empty()) {
        return Err(ScreenerError::InvalidFilter {
            message: "classification filter requires instrument classification metadata".to_owned(),
        });
    }
    match group.operator {
        LogicalOp::And => {
            for item in &group.items {
                let matched = match item {
                    ScreenerFilterItem::Predicate(p) => evaluate_predicate(p, bars)?,
                    ScreenerFilterItem::Group(nested) => evaluate_group(nested, bars)?,
                };
                if !matched {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        LogicalOp::Or => {
            if group.items.is_empty() {
                return Ok(true);
            }
            for item in &group.items {
                let matched = match item {
                    ScreenerFilterItem::Predicate(p) => evaluate_predicate(p, bars)?,
                    ScreenerFilterItem::Group(nested) => evaluate_group(nested, bars)?,
                };
                if matched {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}
