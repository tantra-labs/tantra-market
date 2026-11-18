//! Unit and integration tests for `tantra_market_finance::studies`.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_finance::studies::{
    ExperimentalStudyConfig, IntradayBar, MarkovState, OhlcvBar, OrbConfig, OutlierStatus,
    PeriodScheduleKind, ReturnCalcKind, RollBasisMode, SpreadCostConfig, StreakDirection,
    StudyAssetClass, SubCycleKind, analyze_cross_asset_correlation, analyze_cycle_progress,
    analyze_gaps, analyze_intraday_movement, analyze_rolling_5day, evaluate_returns_study,
    run_orb15_flip_backtest, slice_periods,
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
    assert_eq!(
        periods[0].end_date,
        NaiveDate::from_ymd_opt(2025, 8, 28).unwrap()
    );
    assert_eq!(
        periods[1].start_date,
        NaiveDate::from_ymd_opt(2025, 8, 29).unwrap()
    );
    assert_eq!(
        periods[1].end_date,
        NaiveDate::from_ymd_opt(2025, 9, 2).unwrap()
    );
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
        dec!(-0.9), // week -4.2%
        dec!(1.1),
        dec!(1.2),
        dec!(1.0),
        dec!(0.9),
        dec!(1.1), // week +5.4%
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
            spread_cost: SpreadCostConfig::default(),
        },
    )
    .expect("study succeeds");

    assert!(report.friction_summary.is_none());
    assert!(report.streaks.summary.excluded_neutral_count >= 1);
    assert!(report.streaks.summary.positive_streak_count >= 1);
    assert!(report.streaks.summary.negative_streak_count >= 1);
    assert!(matches!(
        report.streaks.summary.current_streak_type,
        Some(StreakDirection::Positive | StreakDirection::Negative)
    ));

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

    let pcts = &report.distributions.percentiles;
    assert!(pcts.p99 >= pcts.p95);
    assert!(pcts.p95 >= pcts.p75);
    assert!(pcts.p75 >= pcts.p50);
    assert!(pcts.p50 >= pcts.p25);
    assert!(pcts.p25 >= pcts.p5);
    assert!(pcts.p5 >= pcts.p1);

    let levels = &report.distributions.probability_levels;
    assert_eq!(levels.len(), 11);
    assert!(levels[0].implied_price >= levels[levels.len() - 1].implied_price);

    let coarse_sum: usize = report
        .distributions
        .coarse_buckets
        .iter()
        .map(|b| b.count)
        .sum();
    assert_eq!(coarse_sum, report.period_series.len());

    assert!(report.distributions.outliers.kpis.total_outliers >= 2);
    assert!(report.distributions.outliers.outlier_table.iter().all(|o| {
        o.status == OutlierStatus::UpsideOutlier || o.status == OutlierStatus::DownsideOutlier
    }));
    for w in report.distributions.outliers.outlier_table.windows(2) {
        assert!(w[0].return_pct.abs() >= w[1].return_pct.abs());
    }

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
            spread_cost: SpreadCostConfig::default(),
        },
    )
    .expect("week+1 high-to-low succeeds");
    assert_eq!(plus_one.period_series.len() + 1, report.period_series.len());
    assert!(
        plus_one
            .period_series
            .iter()
            .all(|p| p.return_pct >= Decimal::ZERO)
    );
}

#[test]
fn fno_spread_basis_and_statutory_costs_reconcile_gross_vs_net() {
    let cal = NseCalendar::from_holidays(vec![]);
    let bars = generate_trading_bars(
        NaiveDate::from_ymd_opt(2025, 1, 3).unwrap(),
        90,
        &[dec!(0.5), dec!(0.3), dec!(-0.2), dec!(0.4)],
    );

    let gross_report = evaluate_returns_study(
        &bars,
        &cal,
        &ExperimentalStudyConfig {
            symbol: "NFO:RELIANCE".to_string(),
            asset_class: StudyAssetClass::FnoStock,
            schedule: PeriodScheduleKind::WeeklyExpiry,
            sub_cycle: SubCycleKind::Current,
            calc_kind: ReturnCalcKind::OpenToClose,
            ma_window: 5,
            outlier_coverage_pct: dec!(90),
            tail_threshold_pct: dec!(2),
            live_spot_override: None,
            spread_cost: SpreadCostConfig::default(),
        },
    )
    .expect("gross report succeeds");

    let net_report = evaluate_returns_study(
        &bars,
        &cal,
        &ExperimentalStudyConfig {
            symbol: "NFO:RELIANCE".to_string(),
            asset_class: StudyAssetClass::FnoStock,
            schedule: PeriodScheduleKind::WeeklyExpiry,
            sub_cycle: SubCycleKind::Current,
            calc_kind: ReturnCalcKind::OpenToClose,
            ma_window: 5,
            outlier_coverage_pct: dec!(90),
            tail_threshold_pct: dec!(2),
            live_spot_override: None,
            spread_cost: SpreadCostConfig {
                mode: RollBasisMode::NetAfterSpreadAndCosts,
                spread_slippage_bps: dec!(10),
                futures_roll_basis_bps: dec!(4),
                include_statutory_costs: true,
            },
        },
    )
    .expect("net-of-spread report succeeds");

    let fr = net_report
        .friction_summary
        .as_ref()
        .expect("friction summary populated");
    assert_eq!(fr.gross_avg_return_pct, gross_report.kpis.avg_return_pct);
    assert!(fr.per_period_spread_bps == dec!(10.00));
    assert!(fr.per_period_statutory_bps > Decimal::ZERO);
    assert!(fr.per_period_basis_bps == dec!(4.00));
    assert!(net_report.kpis.avg_return_pct < gross_report.kpis.avg_return_pct);
    assert!(fr.cumulative_net_delta_pct < Decimal::ZERO);
}

#[test]
fn phase2_intraday_orb_gap_five_day_progress_and_correlation_work_in_decimal() {
    let cal = NseCalendar::from_holidays(vec![]);
    let daily = generate_trading_bars(
        NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
        45,
        &[dec!(0.8), dec!(-0.6), dec!(1.1), dec!(-0.4), dec!(0.5)],
    );
    let peer = generate_trading_bars(
        NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
        45,
        &[dec!(0.6), dec!(-0.4), dec!(0.9), dec!(-0.3), dec!(0.4)],
    );

    // 1. Gaps
    let gaps = analyze_gaps(&daily, &[]);
    assert_eq!(gaps.total_sessions, daily.len() - 1);
    let gap_bucket_sum: usize = gaps.buckets.iter().map(|b| b.count).sum();
    assert_eq!(gap_bucket_sum, gaps.total_sessions);

    // 2. Rolling 5-Day
    let five_d = analyze_rolling_5day(&daily, dec!(1.5));
    assert_eq!(five_d.sample_count, daily.len() - 5);
    assert!(five_d.p95_pct >= five_d.p50_pct);
    assert!(five_d.p50_pct >= five_d.p5_pct);

    // 3. Cycle Progress (0-100%)
    let (periods, _) = slice_periods(
        &daily,
        "NSE:NIFTY50",
        StudyAssetClass::Index,
        PeriodScheduleKind::WeeklyExpiry,
        SubCycleKind::Current,
        ReturnCalcKind::OpenToClose,
        &cal,
    );
    let prog = analyze_cycle_progress(&daily, &periods);
    assert_eq!(prog.deciles.len(), 11);
    assert!(prog.cycle_count >= 5);
    assert!(prog.deciles[5].p90_pct >= prog.deciles[5].p10_pct);

    // 4. Synthetic 5m intraday bars for a session that breaks OR_high then reverses OR_low
    let d1 = NaiveDate::from_ymd_opt(2025, 3, 3).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2025, 3, 4).unwrap();
    let bars_5m = vec![
        // Session 1: OR [100..102], breakout up to 104, then plunges through 99 -> flip short -> closes 97
        IntradayBar {
            date: d1,
            minute_of_day: 555,
            open: dec!(100),
            high: dec!(102),
            low: dec!(100),
            close: dec!(101),
            volume: dec!(1000),
        },
        IntradayBar {
            date: d1,
            minute_of_day: 560,
            open: dec!(101),
            high: dec!(102),
            low: dec!(100),
            close: dec!(101.5),
            volume: dec!(1200),
        },
        IntradayBar {
            date: d1,
            minute_of_day: 570,
            open: dec!(101.5),
            high: dec!(103.5),
            low: dec!(101.2),
            close: dec!(103),
            volume: dec!(1500),
        },
        IntradayBar {
            date: d1,
            minute_of_day: 585,
            open: dec!(103),
            high: dec!(103),
            low: dec!(98.5),
            close: dec!(99),
            volume: dec!(2000),
        },
        IntradayBar {
            date: d1,
            minute_of_day: 600,
            open: dec!(99),
            high: dec!(99.5),
            low: dec!(96.5),
            close: dec!(97),
            volume: dec!(1800),
        },
        // Session 2: pure upside trend above OR_high
        IntradayBar {
            date: d2,
            minute_of_day: 555,
            open: dec!(100),
            high: dec!(101),
            low: dec!(99.5),
            close: dec!(100.5),
            volume: dec!(900),
        },
        IntradayBar {
            date: d2,
            minute_of_day: 570,
            open: dec!(100.5),
            high: dec!(102.5),
            low: dec!(100.5),
            close: dec!(102),
            volume: dec!(1400),
        },
        IntradayBar {
            date: d2,
            minute_of_day: 600,
            open: dec!(102),
            high: dec!(104),
            low: dec!(101.8),
            close: dec!(103.5),
            volume: dec!(1600),
        },
    ];

    let intraday = analyze_intraday_movement(&bars_5m);
    assert!(intraday.available);
    assert_eq!(intraday.total_sessions, 2);
    assert!(!intraday.slots_15m.is_empty());

    let orb = run_orb15_flip_backtest(
        &bars_5m,
        StudyAssetClass::FnoStock,
        "NFO:RELIANCE",
        &OrbConfig {
            opening_range_minutes: 15,
            max_flips_per_day: 2,
            stop_loss_pct: Decimal::ZERO,
            target_pct: Decimal::ZERO,
            spread_slippage_bps: dec!(10),
            include_statutory_costs: true,
        },
    );
    assert!(orb.available);
    // Session 1: long stopped & flipped to short -> 2 trades; Session 2: long held -> 1 trade = 3 trades
    assert_eq!(orb.total_trades, 3);
    assert!(orb.total_friction_pct > Decimal::ZERO);
    assert_eq!(
        orb.net_return_pct,
        (orb.gross_return_pct - orb.total_friction_pct).round_dp(2)
    );

    // 5. Cross-Asset Correlation
    let corr = analyze_cross_asset_correlation(
        &[
            ("NSE:NIFTY50".to_string(), daily),
            ("NSE:BANKNIFTY".to_string(), peer),
        ],
        "NSE:NIFTY50",
        "NSE:BANKNIFTY",
        15,
    );
    assert_eq!(corr.pearson_matrix.len(), 2);
    assert_eq!(corr.pearson_matrix[0][0], dec!(1));
    assert_eq!(corr.pearson_matrix[0][1], corr.pearson_matrix[1][0]);
    assert!(corr.pair_pearson > dec!(0.9));
    assert!(!corr.rolling_series.is_empty());
}
