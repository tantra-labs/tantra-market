//! Unit and integration tests for `tantra_market_finance::studies`.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_finance::studies::{
    ExperimentalStudyConfig, MarkovState, OhlcvBar, OutlierStatus, PeriodScheduleKind,
    ReturnCalcKind, StreakDirection, StudyAssetClass, SubCycleKind, evaluate_returns_study,
    slice_periods,
};
use tantra_market_finance::{MarketCalendar, NseCalendar, Session};

struct FixedHolidayCalendar {
    holidays: Vec<NaiveDate>,
}

impl MarketCalendar for FixedHolidayCalendar {
    fn session(&self) -> Session {
        Session::regular()
    }
    fn is_holiday(&self, date: NaiveDate) -> bool {
        self.holidays.contains(&date)
    }
}

fn generate_trading_bars(start: NaiveDate, days: usize, step_pcts: &[Decimal]) -> Vec<OhlcvBar> {
    let cal = NseCalendar::from_holidays(vec![]);
    let mut bars = Vec::with_capacity(days);
    let mut d = start;
    let mut price = dec!(20000.00);
    let mut i = 0usize;
    while bars.len() < days {
        if cal.is_trading_day(d) {
            let pct = step_pcts[i % step_pcts.len()];
            let open = price;
            let close = (open * (Decimal::ONE + pct / dec!(100))).round_dp(2);
            let high = open.max(close) * dec!(1.005);
            let low = open.min(close) * dec!(0.995);
            bars.push(OhlcvBar {
                date: d,
                open,
                high: high.round_dp(2),
                low: low.round_dp(2),
                close,
                volume: dec!(100000),
                dividend: Decimal::ZERO,
                split_factor: Decimal::ZERO,
            });
            price = close;
            i += 1;
        }
        d += Duration::days(1);
    }
    bars
}

#[test]
fn nse_sep_2025_weekly_cutover_switches_from_thu_to_tue_cleanly() {
    let cal = NseCalendar::from_holidays(vec![]);
    // August 22, 2025 (Fri) -> Sep 16, 2025 (Tue): crosses Sep 3 cutover.
    let bars = generate_trading_bars(
        NaiveDate::from_ymd_opt(2025, 8, 22).unwrap(),
        20,
        &[dec!(0.4), dec!(-0.2), dec!(0.6)],
    );
    let (periods, note) = slice_periods(
        &bars,
        "NSE:NIFTY50",
        StudyAssetClass::Index,
        PeriodScheduleKind::WeeklyExpiry,
        SubCycleKind::Current,
        ReturnCalcKind::OpenToClose,
        &cal,
    );

    assert!(
        note.is_some(),
        "expected regime cutover note across Sep 3, 2025"
    );
    // Week 1: Fri Aug 22 .. Thu Aug 28 -> Thursday expiry
    assert_eq!(
        periods[0].end_date,
        NaiveDate::from_ymd_opt(2025, 8, 28).unwrap()
    );
    // Transition Week 2: Fri Aug 29 .. Tue Sep 2 -> Tuesday transition expiry
    assert_eq!(
        periods[1].start_date,
        NaiveDate::from_ymd_opt(2025, 8, 29).unwrap()
    );
    assert_eq!(
        periods[1].end_date,
        NaiveDate::from_ymd_opt(2025, 9, 2).unwrap()
    );
    // Post-cutover Week 3: Wed Sep 3 .. Tue Sep 9 -> Wed->Tue
    assert_eq!(
        periods[2].start_date,
        NaiveDate::from_ymd_opt(2025, 9, 3).unwrap()
    );
    assert_eq!(
        periods[2].end_date,
        NaiveDate::from_ymd_opt(2025, 9, 9).unwrap()
    );
}

#[test]
fn holiday_on_thursday_pulls_expiry_to_wednesday() {
    let thu_holiday = NaiveDate::from_ymd_opt(2025, 8, 14).unwrap();
    let cal = FixedHolidayCalendar {
        holidays: vec![thu_holiday],
    };
    let bars = generate_trading_bars(
        NaiveDate::from_ymd_opt(2025, 8, 8).unwrap(),
        12,
        &[dec!(0.5), dec!(0.3)],
    );
    let (periods, _) = slice_periods(
        &bars,
        "NIFTY 50",
        StudyAssetClass::Index,
        PeriodScheduleKind::WeeklyExpiry,
        SubCycleKind::Current,
        ReturnCalcKind::OpenToClose,
        &cal,
    );
    // Aug 8 (Fri) .. Aug 13 (Wed) since Aug 14 (Thu) is a holiday
    assert_eq!(
        periods[0].end_date,
        NaiveDate::from_ymd_opt(2025, 8, 13).unwrap()
    );
    assert_eq!(periods[0].end_date.weekday(), Weekday::Wed);
}

#[test]
fn multi_asset_schedules_cover_etf_fx_and_commodities() {
    let cal = NseCalendar::from_holidays(vec![]);
    let bars = generate_trading_bars(
        NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
        45,
        &[dec!(0.7), dec!(-0.5), dec!(0.2), dec!(-0.1), dec!(0.8)],
    );

    // Calendar Weekly (ETFs/Equities): closes on Friday (or last trading day of week)
    let (etf_weekly, etf_note) = slice_periods(
        &bars,
        "NSE:NIFTYBEES",
        StudyAssetClass::Etf,
        PeriodScheduleKind::CalendarWeekly,
        SubCycleKind::Current,
        ReturnCalcKind::CloseToClose,
        &cal,
    );
    assert!(etf_note.is_none());
    assert!(etf_weekly.len() >= 8);

    // MCX Commodity 20th-roll cycle
    let (mcx_cycles, _) = slice_periods(
        &bars,
        "MCX:GOLD",
        StudyAssetClass::Commodity,
        PeriodScheduleKind::McxExpiryCycle,
        SubCycleKind::Current,
        ReturnCalcKind::OpenToClose,
        &cal,
    );
    assert!(mcx_cycles.len() >= 2);
    assert!(mcx_cycles[0].end_date.day() <= 20);
}

#[test]
fn evaluate_returns_study_verifies_streaks_markov_quantiles_and_outliers() {
    let cal = NseCalendar::from_holidays(vec![]);
    // Create 180 daily bars (~36 weekly cycles) with distinct positive/negative runs & tail swings
    let pattern = [
        dec!(0.5),
        dec!(0.6),
        dec!(0.4),
        dec!(0.2),
        dec!(0.3), // week +2%
        dec!(0.4),
        dec!(0.4),
        dec!(0.3),
        dec!(0.3),
        dec!(0.2), // week +1.6%
        dec!(-0.6),
        dec!(-0.5),
        dec!(-0.7),
        dec!(-0.5),
        dec!(-0.4), // week -2.7%
        dec!(-0.9),
        dec!(-0.9),
        dec!(-0.8),
        dec!(-0.8),
        dec!(-0.9), // week -4.2% (tail neg)
        dec!(1.1),
        dec!(1.2),
        dec!(1.0),
        dec!(0.9),
        dec!(1.1), // week +5.4% (tail pos)
        dec!(0.0),
        dec!(0.0),
        dec!(0.0),
        dec!(0.0),
        dec!(0.0), // neutral week (0%)
        dec!(-0.2),
        dec!(-0.1),
        dec!(-0.2),
        dec!(-0.1),
        dec!(-0.2), // week -0.8%
    ];
    let bars = generate_trading_bars(NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(), 175, &pattern);

    let report = evaluate_returns_study(
        &bars,
        &cal,
        &ExperimentalStudyConfig {
            symbol: "NIFTY 50".to_string(),
            asset_class: StudyAssetClass::Index,
            schedule: PeriodScheduleKind::WeeklyExpiry,
            sub_cycle: SubCycleKind::Current,
            calc_kind: ReturnCalcKind::OpenToClose,
            ma_window: 5,
            outlier_coverage_pct: dec!(80),
            tail_threshold_pct: dec!(2),
            live_spot_override: None,
        },
    )
    .expect("study succeeds");

    // 1. Neutral 0% weeks excluded from streak run lengths cleanly
    assert!(report.streaks.summary.excluded_neutral_count >= 1);
    assert!(report.streaks.summary.positive_streak_count >= 1);
    assert!(report.streaks.summary.negative_streak_count >= 1);
    assert!(matches!(
        report.streaks.summary.current_streak_type,
        Some(StreakDirection::Positive | StreakDirection::Negative)
    ));

    // 2. Markov steady state probabilities sum to ~100% and rows represent all 6 states
    assert_eq!(report.markov.transition_matrix.len(), 6);
    assert_eq!(report.markov.steady_state.len(), 6);
    assert_eq!(
        report.markov.steady_state[0].state,
        MarkovState::ShortPositive
    );
    let ss_sum: Decimal = report
        .markov
        .steady_state
        .iter()
        .map(|s| s.probability_pct)
        .sum();
    assert!(
        (ss_sum - dec!(100)).abs() <= dec!(0.15),
        "steady state sum = {ss_sum}"
    );

    // 3. Monotonicity of empirical percentiles & Probability-Based Levels
    let pcts = &report.distributions.percentiles;
    assert!(pcts.p99 >= pcts.p95);
    assert!(pcts.p95 >= pcts.p75);
    assert!(pcts.p75 >= pcts.p50);
    assert!(pcts.p50 >= pcts.p25);
    assert!(pcts.p25 >= pcts.p5);
    assert!(pcts.p5 >= pcts.p1);

    let levels = &report.distributions.probability_levels;
    assert_eq!(levels.len(), 11);
    // Higher prob of closing ABOVE -> lower implied spot threshold
    assert!(levels[0].implied_price >= levels[levels.len() - 1].implied_price);

    // 4. Coarse buckets sum to total periods
    let coarse_sum: usize = report
        .distributions
        .coarse_buckets
        .iter()
        .map(|b| b.count)
        .sum();
    assert_eq!(coarse_sum, report.period_series.len());

    // 5. Survivability outliers exist under 80% coverage and are sorted by |ret| descending
    assert!(report.distributions.outliers.kpis.total_outliers >= 2);
    assert!(report.distributions.outliers.outlier_table.iter().all(|o| {
        o.status == OutlierStatus::UpsideOutlier || o.status == OutlierStatus::DownsideOutlier
    }));
    for w in report.distributions.outliers.outlier_table.windows(2) {
        assert!(w[0].return_pct.abs() >= w[1].return_pct.abs());
    }

    // 6. Week+1 carry mode shortens horizon by 1
    let plus_one = evaluate_returns_study(
        &bars,
        &cal,
        &ExperimentalStudyConfig {
            symbol: "NIFTY 50".to_string(),
            asset_class: StudyAssetClass::Index,
            schedule: PeriodScheduleKind::WeeklyExpiry,
            sub_cycle: SubCycleKind::PlusOne,
            calc_kind: ReturnCalcKind::HighToLow,
            ma_window: 5,
            outlier_coverage_pct: dec!(90),
            tail_threshold_pct: dec!(2),
            live_spot_override: Some(dec!(24000)),
        },
    )
    .expect("week+1 high-to-low succeeds");
    assert_eq!(plus_one.period_series.len() + 1, report.period_series.len());
    // High-to-Low is always non-negative
    assert!(
        plus_one
            .period_series
            .iter()
            .all(|p| p.return_pct >= Decimal::ZERO)
    );
}
