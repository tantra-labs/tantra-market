//! Regime-aware period boundary slicer across multi-asset trading calendars.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::calendar::MarketCalendar;

/// Generalized asset class supported by the `Experimental-Analysis` workbench.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyAssetClass {
    /// Cash / benchmark equity indices (e.g. `NIFTY 50`, `BANK NIFTY`, `SENSEX`).
    Index,
    /// F&O eligible stocks and continuous futures.
    FnoStock,
    /// Exchange-traded funds.
    Etf,
    /// Cash equities.
    Equity,
    /// Foreign exchange pairs (24h or session-aligned).
    Forex,
    /// Commodities (MCX / global futures).
    Commodity,
}

/// Period slicing convention mapped across asset classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodScheduleKind {
    /// NSE / BSE weekly derivative expiry cycle (pre/post Sep 3, 2025 cutover aware).
    WeeklyExpiry,
    /// Overlapping `Week + 1` carry window (`cycle_k.open` → `cycle_{k+1}.close`).
    WeekPlusOne,
    /// Monthly derivative expiry cycle (last designated expiry trading day of month).
    MonthlyExpiry,
    /// Standard Monday-to-Friday calendar week (trading days within ISO week).
    CalendarWeekly,
    /// Standard calendar month (first to last trading day in month).
    CalendarMonthly,
    /// Monthly front-month contract cycle (last trading day of month).
    FrontMonthCycle,
    /// MCX-style commodity roll cycle (ending on or before the 20th trading day).
    McxExpiryCycle,
    /// Single daily trading bar.
    Daily,
}

/// Sub-cycle horizon modifier applied inside `Returns Analysis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubCycleKind {
    /// Current expiry / period (`P_k`).
    #[default]
    Current,
    /// Carry through next period (`P_k.open` → `P_{k+1}.close`).
    PlusOne,
}

/// Empirical return formula across a sliced trading period.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReturnCalcKind {
    /// Open-to-Close: `(C_end - O_start) / O_start * 100`.
    #[default]
    OpenToClose,
    /// High-to-Low range percentage: `(max(H) - min(L)) / O_start * 100`.
    HighToLow,
    /// Close-to-Close (including overnight/inter-cycle gaps): `(C_end - C_prev) / C_prev * 100`.
    CloseToClose,
}

/// Single daily / session OHLCV observation in ascending date order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OhlcvBar {
    /// Session trading date (exchange local calendar date).
    pub date: NaiveDate,
    /// Opening price.
    pub open: Decimal,
    /// Highest traded price.
    pub high: Decimal,
    /// Lowest traded price.
    pub low: Decimal,
    /// Closing price.
    pub close: Decimal,
    /// Session volume.
    pub volume: Decimal,
    /// Cash dividend on ex-date (zero if none).
    pub dividend: Decimal,
    /// Stock split ratio on ex-date (zero or 1 if none; preserved verbatim for display).
    pub split_factor: Decimal,
}

/// One sliced trading cycle ready for statistical analysis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlicedPeriod {
    /// 1-based ordinal index in the selected date window.
    pub index: usize,
    /// First trading bar date in the cycle.
    pub start_date: NaiveDate,
    /// Last trading bar date in the cycle.
    pub end_date: NaiveDate,
    /// Opening price of the first bar in the cycle.
    pub entry_price: Decimal,
    /// Maximum high across bars in the cycle.
    pub high_price: Decimal,
    /// Minimum low across bars in the cycle.
    pub low_price: Decimal,
    /// Closing price of the last bar in the cycle.
    pub exit_price: Decimal,
    /// Closing price of the preceding cycle (or `entry_price` for the first period).
    pub prev_close: Decimal,
    /// Computed percentage return under the active [`ReturnCalcKind`].
    pub return_pct: Decimal,
    /// Human-readable regime/schedule tag (e.g. `"Fri→Thu"` or `"Wed→Tue"`).
    pub regime_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstrumentExpiryFamily {
    Nifty,
    BankNifty,
    FinNifty,
    MidcpNifty,
    Sensex,
    GenericNse,
}

fn classify_expiry_family(symbol: &str) -> InstrumentExpiryFamily {
    let upper = symbol.trim().to_ascii_uppercase();
    if upper.contains("BANKNIFTY") || upper.contains("BANK NIFTY") {
        InstrumentExpiryFamily::BankNifty
    } else if upper.contains("FINNIFTY") || upper.contains("FIN NIFTY") {
        InstrumentExpiryFamily::FinNifty
    } else if upper.contains("MIDCPNIFTY") || upper.contains("MIDCAP") {
        InstrumentExpiryFamily::MidcpNifty
    } else if upper.contains("SENSEX") {
        InstrumentExpiryFamily::Sensex
    } else if upper.contains("NIFTY") {
        InstrumentExpiryFamily::Nifty
    } else {
        InstrumentExpiryFamily::GenericNse
    }
}

/// NSE SEBI Sep 3, 2025 cutover (`2025-09-02` closes the transition week; `2025-09-03` opens `Wed→Tue`).
const NSE_CUTOVER_DATE: NaiveDate = match NaiveDate::from_ymd_opt(2025, 9, 3) {
    Some(d) => d,
    None => unreachable!(),
};

/// Returns the target expiry weekday and display label for an instrument on `date`.
fn expiry_weekday_for(family: InstrumentExpiryFamily, date: NaiveDate) -> (Weekday, &'static str) {
    match family {
        InstrumentExpiryFamily::Sensex => {
            let cut_2025_jan = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
            if date >= NSE_CUTOVER_DATE {
                (Weekday::Thu, "Fri→Thu (BSE Thu Expiry)")
            } else if date >= cut_2025_jan {
                (Weekday::Tue, "Wed→Tue (BSE Tue Expiry)")
            } else {
                (Weekday::Fri, "Mon→Fri (BSE Fri Expiry)")
            }
        }
        InstrumentExpiryFamily::BankNifty => {
            let bnf_wed_start = NaiveDate::from_ymd_opt(2023, 9, 4).unwrap();
            let bnf_monthly_only = NaiveDate::from_ymd_opt(2024, 11, 14).unwrap();
            if date >= NSE_CUTOVER_DATE {
                (Weekday::Tue, "Wed→Tue (Tue Expiry)")
            } else if date >= bnf_monthly_only {
                (Weekday::Thu, "Fri→Thu (Thu Expiry)")
            } else if date >= bnf_wed_start {
                (Weekday::Wed, "Thu→Wed (Wed Expiry)")
            } else {
                (Weekday::Thu, "Fri→Thu (Thu Expiry)")
            }
        }
        InstrumentExpiryFamily::FinNifty => {
            let sebi_rationalization = NaiveDate::from_ymd_opt(2024, 11, 14).unwrap();
            if date >= NSE_CUTOVER_DATE || date < sebi_rationalization {
                (Weekday::Tue, "Wed→Tue (Tue Expiry)")
            } else {
                (Weekday::Thu, "Fri→Thu (Thu Expiry)")
            }
        }
        InstrumentExpiryFamily::MidcpNifty => {
            if date >= NSE_CUTOVER_DATE {
                (Weekday::Tue, "Wed→Tue (Tue Expiry)")
            } else if date < NaiveDate::from_ymd_opt(2024, 11, 14).unwrap() {
                (Weekday::Mon, "Tue→Mon (Mon Expiry)")
            } else {
                (Weekday::Thu, "Fri→Thu (Thu Expiry)")
            }
        }
        InstrumentExpiryFamily::Nifty | InstrumentExpiryFamily::GenericNse => {
            // Transition bridge: Friday Aug 29, 2025 -> Tuesday Sep 2, 2025 closes right
            // before Wednesday Sep 3, 2025 opens the new Wed->Tue cycle cleanly.
            let bridge_start = NaiveDate::from_ymd_opt(2025, 8, 29).unwrap();
            if date >= NSE_CUTOVER_DATE {
                (Weekday::Tue, "Wed→Tue (Tue Expiry)")
            } else if date >= bridge_start {
                (Weekday::Tue, "Fri→Tue (NSE Sep 2025 Transition)")
            } else {
                (Weekday::Thu, "Fri→Thu (Thu Expiry)")
            }
        }
    }
}

fn next_or_same_weekday(date: NaiveDate, target: Weekday) -> NaiveDate {
    let mut d = date;
    while d.weekday() != target {
        d += Duration::days(1);
    }
    d
}

fn last_weekday_of_month(year: i32, month: u32, target: Weekday) -> NaiveDate {
    let next_month = if month == 12 { 1 } else { month + 1 };
    let next_year = if month == 12 { year + 1 } else { year };
    let first_of_next = NaiveDate::from_ymd_opt(next_year, next_month, 1).unwrap();
    let mut d = first_of_next - Duration::days(1);
    while d.weekday() != target {
        d -= Duration::days(1);
    }
    d
}

fn adjust_backward_for_holiday(date: NaiveDate, calendar: &dyn MarketCalendar) -> NaiveDate {
    let mut d = date;
    for _ in 0..14 {
        if calendar.is_trading_day(d) {
            return d;
        }
        d -= Duration::days(1);
    }
    date
}

/// Computes the canonical grouping `(cycle_key_date, regime_label)` for an observation `date`.
fn period_key_for_date(
    date: NaiveDate,
    family: InstrumentExpiryFamily,
    schedule: PeriodScheduleKind,
    calendar: &dyn MarketCalendar,
) -> (NaiveDate, String) {
    match schedule {
        PeriodScheduleKind::Daily => (date, "Daily Session".to_string()),
        PeriodScheduleKind::CalendarWeekly => {
            let mut sun = date;
            while sun.weekday() != Weekday::Sun {
                sun += Duration::days(1);
            }
            (sun, "Mon→Fri (Calendar Weekly)".to_string())
        }
        PeriodScheduleKind::CalendarMonthly | PeriodScheduleKind::FrontMonthCycle => {
            let next_m = if date.month() == 12 {
                1
            } else {
                date.month() + 1
            };
            let next_y = if date.month() == 12 {
                date.year() + 1
            } else {
                date.year()
            };
            let month_end = NaiveDate::from_ymd_opt(next_y, next_m, 1).unwrap() - Duration::days(1);
            let label = if schedule == PeriodScheduleKind::FrontMonthCycle {
                "Front-Month Cycle"
            } else {
                "Calendar Monthly"
            };
            (month_end, label.to_string())
        }
        PeriodScheduleKind::McxExpiryCycle => {
            // Cycle rolls on the 20th of each month (if date > 20th, falls into next month's 20th).
            let (target_y, target_m) = if date.day() <= 20 {
                (date.year(), date.month())
            } else if date.month() == 12 {
                (date.year() + 1, 1)
            } else {
                (date.year(), date.month() + 1)
            };
            let nominal = NaiveDate::from_ymd_opt(target_y, target_m, 20).unwrap();
            let adjusted = adjust_backward_for_holiday(nominal, calendar);
            // If holiday adjustment pulled the 20th earlier than `date` (e.g. 20th is Sunday, adjusted to 18th, and `date` is 19th), advance to next month's cycle.
            if date > adjusted {
                let (ny, nm) = if target_m == 12 {
                    (target_y + 1, 1)
                } else {
                    (target_y, target_m + 1)
                };
                let next_nom = NaiveDate::from_ymd_opt(ny, nm, 20).unwrap();
                (
                    adjust_backward_for_holiday(next_nom, calendar),
                    "MCX Roll (20th)".to_string(),
                )
            } else {
                (adjusted, "MCX Roll (20th)".to_string())
            }
        }
        PeriodScheduleKind::WeeklyExpiry | PeriodScheduleKind::WeekPlusOne => {
            let (wd, label) = expiry_weekday_for(family, date);
            let nominal_end = next_or_same_weekday(date, wd);
            let adjusted_end = adjust_backward_for_holiday(nominal_end, calendar);
            // If `date` falls *after* an early-holiday expiry within the same nominal week (e.g., Thu was holiday so expiry shifted to Wed, and bar is on Fri? Next week catches Fri, except if `date > adjusted_end`, shift forward one cycle).
            if date > adjusted_end {
                let next_base = nominal_end + Duration::days(1);
                let (next_wd, next_label) = expiry_weekday_for(family, next_base);
                let next_nom = next_or_same_weekday(next_base, next_wd);
                (
                    adjust_backward_for_holiday(next_nom, calendar),
                    next_label.to_string(),
                )
            } else {
                (adjusted_end, label.to_string())
            }
        }
        PeriodScheduleKind::MonthlyExpiry => {
            let (wd, _) = expiry_weekday_for(
                family,
                NaiveDate::from_ymd_opt(date.year(), date.month(), 15).unwrap(),
            );
            let nominal = last_weekday_of_month(date.year(), date.month(), wd);
            let adjusted = adjust_backward_for_holiday(nominal, calendar);
            if date > adjusted {
                let (ny, nm) = if date.month() == 12 {
                    (date.year() + 1, 1)
                } else {
                    (date.year(), date.month() + 1)
                };
                let (next_wd, _) =
                    expiry_weekday_for(family, NaiveDate::from_ymd_opt(ny, nm, 15).unwrap());
                let next_nom = last_weekday_of_month(ny, nm, next_wd);
                let next_adj = adjust_backward_for_holiday(next_nom, calendar);
                let label = if next_nom >= NSE_CUTOVER_DATE {
                    "Monthly Expiry (Tue)"
                } else {
                    "Monthly Expiry (Thu)"
                };
                (next_adj, label.to_string())
            } else {
                let label = if nominal >= NSE_CUTOVER_DATE {
                    "Monthly Expiry (Tue)"
                } else {
                    "Monthly Expiry (Thu)"
                };
                (adjusted, label.to_string())
            }
        }
    }
}

/// Computes the period percentage return using exact `Decimal` arithmetic.
pub(crate) fn compute_period_return(
    entry_open: Decimal,
    high: Decimal,
    low: Decimal,
    exit_close: Decimal,
    prev_close: Decimal,
    calc_kind: ReturnCalcKind,
) -> Decimal {
    let hundred = Decimal::from(100);
    match calc_kind {
        ReturnCalcKind::OpenToClose => {
            if entry_open.is_zero() {
                Decimal::ZERO
            } else {
                (((exit_close - entry_open) / entry_open) * hundred).round_dp(4)
            }
        }
        ReturnCalcKind::HighToLow => {
            if entry_open.is_zero() {
                Decimal::ZERO
            } else {
                (((high - low) / entry_open) * hundred).round_dp(4)
            }
        }
        ReturnCalcKind::CloseToClose => {
            if prev_close.is_zero() {
                Decimal::ZERO
            } else {
                (((exit_close - prev_close) / prev_close) * hundred).round_dp(4)
            }
        }
    }
}

/// Normalizes default schedules when callers pass generic or asset-class specific options,
/// and slices daily OHLCV bars into chronological periods.
pub fn slice_periods(
    bars: &[OhlcvBar],
    symbol: &str,
    asset_class: StudyAssetClass,
    schedule: PeriodScheduleKind,
    sub_cycle: SubCycleKind,
    calc_kind: ReturnCalcKind,
    calendar: &dyn MarketCalendar,
) -> (Vec<SlicedPeriod>, Option<String>) {
    if bars.is_empty() {
        return (Vec::new(), None);
    }

    let mut sorted = bars.to_vec();
    sorted.sort_by_key(|b| b.date);

    let effective_schedule = match schedule {
        PeriodScheduleKind::WeekPlusOne => PeriodScheduleKind::WeeklyExpiry,
        other => other,
    };
    let effective_sub_cycle = if schedule == PeriodScheduleKind::WeekPlusOne {
        SubCycleKind::PlusOne
    } else {
        sub_cycle
    };

    let family = classify_expiry_family(symbol);

    #[derive(Clone)]
    struct Bucket {
        key: NaiveDate,
        regime_label: String,
        start_date: NaiveDate,
        end_date: NaiveDate,
        open: Decimal,
        high: Decimal,
        low: Decimal,
        close: Decimal,
    }

    let mut buckets: Vec<Bucket> = Vec::new();
    let mut saw_pre_cutover = false;
    let mut saw_post_cutover = false;

    for bar in &sorted {
        let (key, label) = period_key_for_date(bar.date, family, effective_schedule, calendar);
        if bar.date < NSE_CUTOVER_DATE {
            saw_pre_cutover = true;
        } else {
            saw_post_cutover = true;
        }
        if let Some(last) = buckets.last_mut()
            && last.key == key
        {
            last.end_date = bar.date;
            if bar.high > last.high {
                last.high = bar.high;
            }
            if bar.low < last.low {
                last.low = bar.low;
            }
            last.close = bar.close;
            continue;
        }
        buckets.push(Bucket {
            key,
            regime_label: label,
            start_date: bar.date,
            end_date: bar.date,
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
        });
    }

    let regime_note = if matches!(
        asset_class,
        StudyAssetClass::Index | StudyAssetClass::FnoStock
    ) && matches!(
        effective_schedule,
        PeriodScheduleKind::WeeklyExpiry | PeriodScheduleKind::MonthlyExpiry
    ) && saw_pre_cutover
        && saw_post_cutover
    {
        Some(
            "NSE/SEBI Regime Cutover (Sep 3, 2025): NIFTY weekly expiry transitioned from Thursday (Fri→Thu) to Tuesday (Wed→Tue); SENSEX transitioned from Tue to Thu."
                .to_string(),
        )
    } else {
        None
    };

    let mut periods = Vec::with_capacity(buckets.len());

    match effective_sub_cycle {
        SubCycleKind::Current => {
            for (idx, b) in buckets.iter().enumerate() {
                let prev_close = if idx == 0 {
                    b.open
                } else {
                    buckets[idx - 1].close
                };
                let ret =
                    compute_period_return(b.open, b.high, b.low, b.close, prev_close, calc_kind);
                periods.push(SlicedPeriod {
                    index: idx + 1,
                    start_date: b.start_date,
                    end_date: b.end_date,
                    entry_price: b.open,
                    high_price: b.high,
                    low_price: b.low,
                    exit_price: b.close,
                    prev_close,
                    return_pct: ret,
                    regime_label: b.regime_label.clone(),
                });
            }
        }
        SubCycleKind::PlusOne => {
            if buckets.len() < 2 {
                return (Vec::new(), regime_note);
            }
            for idx in 0..(buckets.len() - 1) {
                let first = &buckets[idx];
                let second = &buckets[idx + 1];
                let high = first.high.max(second.high);
                let low = first.low.min(second.low);
                let prev_close = if idx == 0 {
                    first.open
                } else {
                    buckets[idx - 1].close
                };
                let ret = compute_period_return(
                    first.open,
                    high,
                    low,
                    second.close,
                    prev_close,
                    calc_kind,
                );
                periods.push(SlicedPeriod {
                    index: idx + 1,
                    start_date: first.start_date,
                    end_date: second.end_date,
                    entry_price: first.open,
                    high_price: high,
                    low_price: low,
                    exit_price: second.close,
                    prev_close,
                    return_pct: ret,
                    regime_label: format!("{} (+1)", first.regime_label),
                });
            }
        }
    }

    (periods, regime_note)
}
