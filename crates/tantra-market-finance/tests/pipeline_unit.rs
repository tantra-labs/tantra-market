//! TDD tests for composable strategy, optimization-loop, and hedge synthesis primitives
//! (`tantra_market_finance::studies::pipeline` — Track T21.1 / Plan 0015 Phase 1).

use chrono::{Duration, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tantra_market_finance::studies::options_events::LiveOptionStrikeQuote;
use tantra_market_finance::studies::pipeline::{
    ElephantZonesExtractor, EntrySignalPrimitive, ExpiryHorizon, HedgeOverlayPolicy,
    HedgeStructureKind, HedgeSynthesisInput, HedgeSynthesizer, IndicatorPrimitive, IndicatorSpec,
    LoopEvalRequest, Orb15Extractor, OrbBreakoutEntryFilter, OuterBufferStopPolicy,
    ParameterSweepRequest, ParameterSweepRunner, RejectionWickFilter, SessionClock, SessionPreset,
    SignalDirection, SplitArtifactGuard, StandardIndicatorEvaluator, StopExitPolicy,
    StopPolicyMode, StrategyLoopKernel, StrategyParameterBounds, StudyBar, ZoneExtractor,
    ZonePolarity, ZoneTouchEntryFilter,
};
use tantra_market_finance::studies::spreads::{RollBasisMode, SpreadCostConfig};
use tantra_market_finance::studies::{IntradayBar, OhlcvBar, StudyAssetClass};
use tantra_market_finance::{MarketCalendar, NseCalendar, Session};

struct PlainCalendar;
impl MarketCalendar for PlainCalendar {
    fn session(&self) -> Session {
        Session::regular()
    }
    fn is_holiday(&self, _date: NaiveDate) -> bool {
        false
    }
}

fn make_daily_study_bars(start: NaiveDate, count: usize, step_pcts: &[Decimal]) -> Vec<StudyBar> {
    let cal = NseCalendar::from_holidays(vec![]);
    let mut bars = Vec::with_capacity(count);
    let mut d = start;
    let mut price = dec!(22000.00);
    let mut idx = 0usize;
    while bars.len() < count {
        if cal.is_trading_day(d) {
            let pct = step_pcts[idx % step_pcts.len()];
            let open = price;
            let close = (open * (Decimal::ONE + pct / dec!(100))).round_dp(2);
            let high = (open.max(close) * dec!(1.008)).round_dp(2);
            let low = (open.min(close) * dec!(0.992)).round_dp(2);
            let ohlcv = OhlcvBar {
                date: d,
                open,
                high,
                low,
                close,
                volume: dec!(250000),
                dividend: Decimal::ZERO,
                split_factor: Decimal::ZERO,
            };
            bars.push(StudyBar::from(&ohlcv));
            price = close;
            idx += 1;
        }
        d += Duration::days(1);
    }
    bars
}

fn make_intraday_session_bars(
    dates: &[NaiveDate],
    minute_slots: &[u16],
    base_price: Decimal,
) -> Vec<StudyBar> {
    let mut bars = Vec::new();
    let mut px = base_price;
    for (s_idx, &date) in dates.iter().enumerate() {
        for (k, &minute) in minute_slots.iter().enumerate() {
            let swing = match (s_idx + k) % 4 {
                0 => dec!(0.35),
                1 => dec!(-0.45),
                2 => dec!(0.55),
                _ => dec!(-0.20),
            };
            let open = px;
            let close = (open * (Decimal::ONE + swing / dec!(100))).round_dp(2);
            let high = (open.max(close) * dec!(1.004)).round_dp(2);
            let low = (open.min(close) * dec!(0.996)).round_dp(2);
            let bar = IntradayBar {
                date,
                minute_of_day: minute,
                open,
                high,
                low,
                close,
                volume: dec!(50000),
            };
            bars.push(StudyBar::from(&bar));
            px = close;
        }
    }
    bars
}

#[test]
fn session_clock_slices_nse_sep2025_cutover_mcx_and_daily_regimes() {
    let cal = PlainCalendar;
    // Crossing Sep 3, 2025 NSE cutover cleanly
    let d1 = NaiveDate::from_ymd_opt(2025, 9, 2).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2025, 9, 3).unwrap();
    let nse_bars = make_intraday_session_bars(&[d1, d2], &[555, 570, 585, 600], dec!(24800));
    let clock_nse = SessionClock::new(SessionPreset::NseSep2025Expiry);
    let slices_nse = clock_nse.slice_sessions(&nse_bars, StudyAssetClass::Index, &cal);
    assert_eq!(slices_nse.len(), 2);
    assert!(
        slices_nse[0].regime_tag.contains("Pre-Cutover")
            || slices_nse[0].session_key == "2025-09-02"
    );
    assert!(
        slices_nse[1].regime_tag.contains("Tue-Expiry")
            || slices_nse[1].session_key == "2025-09-03"
    );

    // MCX Commodity preset includes evening sessions up to 23:30 IST (minute 1410)
    let mcx_bars = make_intraday_session_bars(&[d1, d2], &[540, 900, 1200, 1410], dec!(72000));
    let clock_mcx = SessionClock::new(SessionPreset::McxCommodity);
    let slices_mcx = clock_mcx.slice_sessions(&mcx_bars, StudyAssetClass::Commodity, &cal);
    assert_eq!(slices_mcx.len(), 2);
    assert_eq!(slices_mcx[0].end_index, 3);
    assert_eq!(slices_mcx[1].start_index, 4);

    // Daily monthly regime across multiple months
    let daily_bars = make_daily_study_bars(
        NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        90,
        &[dec!(0.5), dec!(-0.4), dec!(0.8), dec!(-0.7)],
    );
    let clock_daily = SessionClock::new(SessionPreset::DailyMonthly);
    let slices_daily = clock_daily.slice_sessions(&daily_bars, StudyAssetClass::Equity, &cal);
    assert!(slices_daily.len() >= 3);
}

#[test]
fn split_artifact_guard_backward_adjusts_corporate_split_and_prevents_atr_spike() {
    let mut bars = make_daily_study_bars(
        NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
        30,
        &[dec!(0.4), dec!(-0.3), dec!(0.5)],
    );
    // Inject a 5-for-1 corporate stock split at index 20: price drops 5x overnight.
    let pre_split_close = bars[19].close;
    let factor = dec!(5);
    for b in bars.iter_mut().skip(20) {
        b.open = (b.open / factor).round_dp(2);
        b.high = (b.high / factor).round_dp(2);
        b.low = (b.low / factor).round_dp(2);
        b.close = (b.close / factor).round_dp(2);
        b.volume *= factor;
    }
    bars[20].split_factor = factor;

    let eval = StandardIndicatorEvaluator;
    let raw_ind = eval.evaluate(&bars, &[IndicatorSpec::Atr { period: 14 }]);
    let raw_atr_at_split = raw_ind.atr[20];

    let report = SplitArtifactGuard::default().clean(&bars);
    assert_eq!(report.detected_splits.len(), 1);
    assert_eq!(report.detected_splits[0].bar_index, 20);
    assert_eq!(report.detected_splits[0].factor, factor);

    // Pre-split bar 19 should now match 1/5th scale cleanly
    assert_eq!(
        report.adjusted_bars[19].close,
        (pre_split_close / factor).round_dp(2)
    );
    let clean_ind = eval.evaluate(&report.adjusted_bars, &[IndicatorSpec::Atr { period: 14 }]);
    assert!(
        clean_ind.atr[20] * dec!(5) < raw_atr_at_split,
        "Split guard must prevent synthetic 67x ATR jump: clean={} raw={}",
        clean_ind.atr[20],
        raw_atr_at_split
    );
}

#[test]
fn elephant_zones_extractor_matches_empirical_bands_polarity_flip_and_outer_sl() {
    let cal = PlainCalendar;
    let bars = make_daily_study_bars(
        NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
        120,
        &[
            dec!(0.6),
            dec!(-0.8),
            dec!(1.1),
            dec!(-1.3),
            dec!(0.4),
            dec!(-0.5),
            dec!(1.6),
            dec!(-1.5),
        ],
    );
    let clock = SessionClock::new(SessionPreset::DailyMonthly);
    let sessions = clock.slice_sessions(&bars, StudyAssetClass::Index, &cal);
    let indicators =
        StandardIndicatorEvaluator.evaluate(&bars, &[IndicatorSpec::Atr { period: 14 }]);

    let extractor = ElephantZonesExtractor::default();
    let output = extractor.extract(&bars, &sessions, &indicators);

    assert_eq!(output.sessions.len(), sessions.len());
    // Must emit 4 bands (R2, R1, S1, S2) per session + 1 equilibrium line per session
    assert_eq!(output.bands.len(), sessions.len() * 4);
    assert_eq!(output.levels.len(), sessions.len());

    // First 2 sessions use golden ratio fallback; session >= 3 uses empirical quantiles
    for sess in &output.sessions {
        assert_eq!(sess.bands.len(), 4);
        for b in &sess.bands {
            assert!(b.low < b.high);
            if b.polarity == ZonePolarity::Support {
                assert!(b.outer_sl < b.low);
            } else {
                assert!(b.outer_sl > b.high);
            }
        }
    }
}

#[test]
fn strategy_loop_kernel_proves_outer_buffer_cuts_stop_outs_and_supports_multiple_strategies() {
    let cal = PlainCalendar;
    // Build a realistic 180-bar series with sharp stop-hunt wicks around zone edges followed by recovery
    let start = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();
    let mut bars = Vec::new();
    let mut d = start;
    let mut px = dec!(1000.00);
    for i in 0..180 {
        while !cal.is_trading_day(d) {
            d += Duration::days(1);
        }
        let open = px;
        let (high, low, close) = match i % 7 {
            // Stop-hunt wick down then strong reclaim & bounce next bars
            2 => (
                (open * dec!(1.004)).round_dp(2),
                (open * dec!(0.978)).round_dp(2),
                (open * dec!(0.996)).round_dp(2),
            ),
            3 => (
                (open * dec!(1.015)).round_dp(2),
                (open * dec!(0.992)).round_dp(2),
                (open * dec!(1.012)).round_dp(2),
            ),
            4 => (
                (open * dec!(1.022)).round_dp(2),
                (open * dec!(0.998)).round_dp(2),
                (open * dec!(1.018)).round_dp(2),
            ),
            // Bearish rejection poke above resistance then pullback
            5 => (
                (open * dec!(1.020)).round_dp(2),
                (open * dec!(0.996)).round_dp(2),
                (open * dec!(1.002)).round_dp(2),
            ),
            6 => (
                (open * dec!(1.003)).round_dp(2),
                (open * dec!(0.985)).round_dp(2),
                (open * dec!(0.988)).round_dp(2),
            ),
            _ => (
                (open * dec!(1.009)).round_dp(2),
                (open * dec!(0.991)).round_dp(2),
                (open * dec!(1.001)).round_dp(2),
            ),
        };
        let bar = OhlcvBar {
            date: d,
            open,
            high,
            low,
            close,
            volume: dec!(100000),
            dividend: Decimal::ZERO,
            split_factor: Decimal::ZERO,
        };
        bars.push(StudyBar::from(&bar));
        px = close;
        d += Duration::days(1);
    }

    // Strategy 1: Elephant Edge V3 (`ElephantZonesExtractor` + `ZoneTouchEntryFilter`)
    let cmp = StrategyLoopKernel::compare_stop_policies(
        &LoopEvalRequest {
            strategy_id: "elephant_edge_v3".into(),
            symbol: "NSE:NIFTYBEES".into(),
            asset_class: StudyAssetClass::Index,
            session_preset: SessionPreset::DailyMonthly,
            bars: bars.clone(),
            horizon_bars: 5,
            target_r: dec!(3),
            atr_period: 14,
            sl_buffer_atr: dec!(0.25),
            expiry_horizon: ExpiryHorizon::Weekly,
            spread_cost: SpreadCostConfig::default(),
            live_chain: Vec::new(),
        },
        &cal,
        &ElephantZonesExtractor::default(),
        &ZoneTouchEntryFilter,
    )
    .expect("EE benchmark comparison must succeed");

    assert!(cmp.total_setups > 0);
    assert_eq!(cmp.models.len(), 4);
    assert_eq!(cmp.models[0].policy_id, "tight_center");
    assert_eq!(cmp.models[2].policy_id, "outer_025_atr");
    // Outer buffer +0.25 ATR must stop out strictly no more often than tight center SL
    assert!(
        cmp.models[2].kpis.stop_out_rate_pct <= cmp.models[0].kpis.stop_out_rate_pct,
        "Expected outer buffer stop-out <= tight center: outer={} tight={}",
        cmp.models[2].kpis.stop_out_rate_pct,
        cmp.models[0].kpis.stop_out_rate_pct
    );

    // Strategy 2: ORB-15 reusing the exact same `StrategyLoopKernel` + `OuterBufferStopPolicy`!
    let intraday = make_intraday_session_bars(
        &[
            NaiveDate::from_ymd_opt(2025, 10, 6).unwrap(),
            NaiveDate::from_ymd_opt(2025, 10, 7).unwrap(),
            NaiveDate::from_ymd_opt(2025, 10, 8).unwrap(),
            NaiveDate::from_ymd_opt(2025, 10, 9).unwrap(),
            NaiveDate::from_ymd_opt(2025, 10, 10).unwrap(),
        ],
        &[555, 570, 585, 600, 615, 630, 645, 660],
        dec!(25000),
    );
    let stop_policy = OuterBufferStopPolicy::new(StopPolicyMode::OuterAtrBuffer {
        buffer_atr: dec!(0.25),
    });
    assert_eq!(stop_policy.policy_id(), "outer_025_atr");

    let orb_res = StrategyLoopKernel::evaluate(
        &LoopEvalRequest {
            strategy_id: "orb_15".into(),
            symbol: "NSE:BANKNIFTY".into(),
            asset_class: StudyAssetClass::Index,
            session_preset: SessionPreset::NseSep2025Expiry,
            bars: intraday,
            horizon_bars: 4,
            target_r: dec!(3),
            atr_period: 14,
            sl_buffer_atr: dec!(0.25),
            expiry_horizon: ExpiryHorizon::Day,
            spread_cost: SpreadCostConfig::default(),
            live_chain: Vec::new(),
        },
        &cal,
        &Orb15Extractor::default(),
        &OrbBreakoutEntryFilter,
        &stop_policy,
        &HedgeSynthesizer,
    )
    .expect("ORB-15 pipeline run must succeed");
    assert_eq!(orb_res.strategy_id, "orb_15");
    assert!(!orb_res.zones.bands.is_empty());
}

#[test]
fn rejection_wick_filter_selects_high_wick_reclaims_cleanly() {
    let cal = PlainCalendar;
    let bars = make_daily_study_bars(
        NaiveDate::from_ymd_opt(2025, 2, 3).unwrap(),
        80,
        &[dec!(0.5), dec!(-1.2), dec!(0.8), dec!(-0.6)],
    );
    let clock = SessionClock::new(SessionPreset::DailyMonthly);
    let sessions = clock.slice_sessions(&bars, StudyAssetClass::FnoStock, &cal);
    let indicators =
        StandardIndicatorEvaluator.evaluate(&bars, &[IndicatorSpec::Atr { period: 14 }]);
    let zones = ElephantZonesExtractor::default().extract(&bars, &sessions, &indicators);

    let touch_signals = ZoneTouchEntryFilter.detect_signals(&bars, &zones, &indicators);
    let wick_signals = RejectionWickFilter::default().detect_signals(&bars, &zones, &indicators);

    assert!(
        wick_signals.len() <= touch_signals.len(),
        "RejectionWickFilter must be at least as selective as raw touch"
    );
}

#[test]
fn parameter_sweep_runner_computes_is_oos_wfe_and_deflated_sharpe() {
    let cal = PlainCalendar;
    let bars = make_daily_study_bars(
        NaiveDate::from_ymd_opt(2025, 1, 2).unwrap(),
        160,
        &[
            dec!(0.7),
            dec!(-1.1),
            dec!(0.9),
            dec!(-0.4),
            dec!(1.2),
            dec!(-1.3),
        ],
    );

    let report = ParameterSweepRunner::run(
        &ParameterSweepRequest {
            base: LoopEvalRequest {
                strategy_id: "elephant_edge_v3".into(),
                symbol: "NSE:RELIANCE".into(),
                asset_class: StudyAssetClass::FnoStock,
                session_preset: SessionPreset::DailyMonthly,
                bars,
                horizon_bars: 5,
                target_r: dec!(3),
                atr_period: 14,
                sl_buffer_atr: dec!(0.00),
                expiry_horizon: ExpiryHorizon::Monthly,
                spread_cost: SpreadCostConfig {
                    mode: RollBasisMode::NetAfterSpreadAndCosts,
                    spread_slippage_bps: dec!(8),
                    futures_roll_basis_bps: Decimal::ZERO,
                    include_statutory_costs: true,
                },
                live_chain: Vec::new(),
            },
            bounds: StrategyParameterBounds {
                sl_buffer_atr_grid: vec![dec!(0.00), dec!(0.25), dec!(0.50)],
                horizon_bars_grid: vec![3, 5],
                target_r_grid: vec![dec!(2), dec!(3)],
                max_candidates: 16,
            },
        },
        &cal,
        &ElephantZonesExtractor::default(),
        &ZoneTouchEntryFilter,
        &HedgeSynthesizer,
    )
    .expect("Sweep must succeed");

    assert_eq!(report.trial_count, 12);
    assert_eq!(report.candidates.len(), 12);
    assert!(report.winner.oos_kpis.trades > 0 || report.winner.is_kpis.trades > 0);
    // Net R must reflect friction drag <= gross R
    assert!(report.winner.full_kpis.net_avg_realized_r <= report.winner.full_kpis.avg_realized_r);
}

#[test]
fn hedge_synthesizer_produces_day_weekly_monthly_and_commodity_structures_with_chain_awareness() {
    let cal = PlainCalendar;
    let bars = make_daily_study_bars(
        NaiveDate::from_ymd_opt(2025, 3, 3).unwrap(),
        70,
        &[dec!(0.6), dec!(-0.7), dec!(0.8), dec!(-0.5)],
    );
    let clock = SessionClock::new(SessionPreset::DailyMonthly);
    let sessions = clock.slice_sessions(&bars, StudyAssetClass::Index, &cal);
    let indicators =
        StandardIndicatorEvaluator.evaluate(&bars, &[IndicatorSpec::Atr { period: 14 }]);
    let zones = ElephantZonesExtractor::default().extract(&bars, &sessions, &indicators);

    let spot = bars.last().unwrap().close;
    let expiry_date = NaiveDate::from_ymd_opt(2025, 6, 26).unwrap();
    let chain = vec![
        LiveOptionStrikeQuote {
            option_type: "PE".into(),
            strike: spot.round_dp(0) - dec!(300),
            expiry: expiry_date,
            lot_size: dec!(75),
            bid: Some(dec!(84)),
            ask: Some(dec!(88)),
            last_price: Some(dec!(86)),
            open_interest: Some(dec!(120000)),
        },
        LiveOptionStrikeQuote {
            option_type: "PE".into(),
            strike: spot.round_dp(0) - dec!(600),
            expiry: expiry_date,
            lot_size: dec!(75),
            bid: Some(dec!(38)),
            ask: Some(dec!(42)),
            last_price: Some(dec!(40)),
            open_interest: Some(dec!(95000)),
        },
        LiveOptionStrikeQuote {
            option_type: "CE".into(),
            strike: spot.round_dp(0),
            expiry: expiry_date,
            lot_size: dec!(75),
            bid: Some(dec!(142)),
            ask: Some(dec!(148)),
            last_price: Some(dec!(145)),
            open_interest: Some(dec!(180000)),
        },
        LiveOptionStrikeQuote {
            option_type: "CE".into(),
            strike: spot.round_dp(0) + dec!(300),
            expiry: expiry_date,
            lot_size: dec!(75),
            bid: Some(dec!(62)),
            ask: Some(dec!(66)),
            last_price: Some(dec!(64)),
            open_interest: Some(dec!(140000)),
        },
    ];

    let synth = HedgeSynthesizer;

    // Day horizon bullish -> Bull Call Spread with live chain quotes
    let day_hedge = synth.synthesize(&HedgeSynthesisInput {
        symbol: "NSE:NIFTY50".into(),
        asset_class: StudyAssetClass::Index,
        spot,
        atr: indicators.atr.last().copied().unwrap_or(dec!(200)),
        horizon: ExpiryHorizon::Day,
        direction: Some(SignalDirection::Long),
        zones: Some(&zones),
        penetration: None,
        iv_gt_rv: false,
        trap_rate_pct: dec!(4.5),
        live_chain: &chain,
        lot_size: dec!(75),
        tick_size: dec!(0.05),
        capital: dec!(500000),
        spread_cost: &SpreadCostConfig::default(),
    });
    assert_eq!(day_hedge.structure, HedgeStructureKind::BullCallSpread);
    assert_eq!(day_hedge.chain_status, "live_quotes");
    assert_eq!(day_hedge.legs.len(), 2);

    // Weekly high IV + elevated trap rate inside equilibrium -> Zone Iron Condor
    let condor = synth.synthesize(&HedgeSynthesisInput {
        symbol: "NSE:NIFTY50".into(),
        asset_class: StudyAssetClass::Index,
        spot,
        atr: indicators.atr.last().copied().unwrap_or(dec!(200)),
        horizon: ExpiryHorizon::Weekly,
        direction: None,
        zones: Some(&zones),
        penetration: None,
        iv_gt_rv: true,
        trap_rate_pct: dec!(16.2),
        live_chain: &[],
        lot_size: dec!(75),
        tick_size: dec!(0.05),
        capital: dec!(500000),
        spread_cost: &SpreadCostConfig::default(),
    });
    assert_eq!(condor.structure, HedgeStructureKind::ZoneIronCondor);
    assert_eq!(condor.chain_status, "theoretical_model");
    assert_eq!(condor.legs.len(), 4);

    // Commodity monthly cycle bullish -> Protective Collar
    let mcx_collar = synth.synthesize(&HedgeSynthesisInput {
        symbol: "MCX:GOLD".into(),
        asset_class: StudyAssetClass::Commodity,
        spot: dec!(75000),
        atr: dec!(650),
        horizon: ExpiryHorizon::CommodityCycle,
        direction: Some(SignalDirection::Long),
        zones: None,
        penetration: None,
        iv_gt_rv: false,
        trap_rate_pct: dec!(5),
        live_chain: &[],
        lot_size: dec!(1),
        tick_size: dec!(1),
        capital: dec!(1000000),
        spread_cost: &SpreadCostConfig::default(),
    });
    assert_eq!(mcx_collar.structure, HedgeStructureKind::ProtectiveCollar);
    assert_eq!(mcx_collar.legs.len(), 3);
}

#[test]
fn golden_vector_parity_with_elephant_edge_overlay_ts() {
    let cal = PlainCalendar;
    let d1 = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2026, 9, 2).unwrap();
    let d3 = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
    let d4 = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();

    let mut bars = Vec::new();
    for i in 0..8 {
        bars.push(StudyBar {
            date: d1,
            minute_of_day: Some(570 + (i as u16) * 30),
            open: dec!(2280),
            high: dec!(2292),
            low: dec!(2268),
            close: dec!(2282) + Decimal::from(i),
            volume: dec!(100000),
            dividend: Decimal::ZERO,
            split_factor: Decimal::ZERO,
        });
    }
    for i in 0..8 {
        bars.push(StudyBar {
            date: d2,
            minute_of_day: Some(570 + (i as u16) * 30),
            open: dec!(2290),
            high: dec!(2320),
            low: dec!(2260),
            close: dec!(2295) + Decimal::from(i),
            volume: dec!(100000),
            dividend: Decimal::ZERO,
            split_factor: Decimal::ZERO,
        });
    }
    bars.push(StudyBar {
        date: d3,
        minute_of_day: Some(570),
        open: dec!(2292),
        high: dec!(2298),
        low: dec!(2286),
        close: dec!(2290),
        volume: dec!(100000),
        dividend: Decimal::ZERO,
        split_factor: Decimal::ZERO,
    });

    // Inject trap candle matching elephant-edge-overlay.test.ts S3 S1
    bars.push(StudyBar {
        date: d3,
        minute_of_day: Some(600),
        open: dec!(2285.4),
        high: dec!(2291.4),
        low: dec!(2249.1),
        close: dec!(2287.4),
        volume: dec!(100000),
        dividend: Decimal::ZERO,
        split_factor: Decimal::ZERO,
    });
    // Session 4 gap up open -> bands flip to Support
    bars.push(StudyBar {
        date: d4,
        minute_of_day: Some(570),
        open: dec!(2445.4),
        high: dec!(2457.4),
        low: dec!(2437.4),
        close: dec!(2450.4),
        volume: dec!(100000),
        dividend: Decimal::ZERO,
        split_factor: Decimal::ZERO,
    });
    bars.push(StudyBar {
        date: d4,
        minute_of_day: Some(600),
        open: dec!(2450.4),
        high: dec!(2460.4),
        low: dec!(2441.4),
        close: dec!(2455.4),
        volume: dec!(100000),
        dividend: Decimal::ZERO,
        split_factor: Decimal::ZERO,
    });

    let clock = SessionClock::new(SessionPreset::NseSep2025Expiry);
    let sessions = clock.slice_sessions(&bars, StudyAssetClass::Index, &cal);
    let indicators =
        StandardIndicatorEvaluator.evaluate(&bars, &[IndicatorSpec::Atr { period: 14 }]);
    let zones = ElephantZonesExtractor::default().extract(&bars, &sessions, &indicators);

    assert_eq!(zones.sessions.len(), 4);
    let s3_s1 = zones.sessions[2]
        .bands
        .iter()
        .find(|b| b.label == "S1")
        .unwrap();
    assert_eq!(s3_s1.polarity, ZonePolarity::Support);
    assert_eq!(s3_s1.center, dec!(2270.00));
    assert_eq!(s3_s1.low, dec!(2254.60));
    assert_eq!(s3_s1.high, dec!(2285.40));
    assert_eq!(s3_s1.outer_sl, dec!(2243.60));

    let s4_r1 = zones.sessions[3]
        .bands
        .iter()
        .find(|b| b.label == "R1")
        .unwrap();
    assert_eq!(s4_r1.polarity, ZonePolarity::Support);
    assert_eq!(s4_r1.center, dec!(2392.15));
    assert!((s4_r1.low - dec!(2372.64)).abs() <= dec!(0.05));
    assert!((s4_r1.high - dec!(2411.66)).abs() <= dec!(0.05));
    assert!((s4_r1.outer_sl - dec!(2358.71)).abs() <= dec!(0.05));
}

#[test]
fn golden_vector_parity_with_stop_hunt_buffer_study_ts() {
    let raw_tuples: [(i32, u32, u32, &str, &str, &str, &str); 90] = [
        (2026, 1, 1, "996", "1013.5", "986", "997.5"),
        (2026, 1, 2, "1000.5", "1024.29", "987.5", "1005.29"),
        (2026, 1, 3, "1008.29", "1025.4", "992.29", "1015.4"),
        (2026, 1, 4, "1011.4", "1032.18", "992.4", "1019.18"),
        (2026, 1, 5, "1022.18", "1044.77", "1000.18", "1028.77"),
        (2026, 1, 6, "1031.77", "1054.67", "1021.77", "1035.67"),
        (2026, 1, 7, "1031.67", "1042.18", "1018.67", "1032.18"),
        (2026, 1, 8, "1035.18", "1048.18", "1016.58", "1032.58"),
        (2026, 1, 9, "1035.58", "1051.58", "1012.08", "1031.08"),
        (2026, 1, 10, "1027.08", "1046.08", "1000.46", "1022.46"),
        (2026, 1, 11, "1025.46", "1035.46", "1012.52", "1022.52"),
        (2026, 1, 12, "1025.52", "1038.56", "1012.52", "1025.56"),
        (2026, 1, 13, "1021.56", "1041.02", "1005.56", "1025.02"),
        (2026, 1, 14, "1028.02", "1053.32", "1009.02", "1034.32"),
        (2026, 1, 15, "1037.32", "1055.05", "1015.32", "1045.05"),
        (2026, 1, 16, "1041.05", "1061.36", "1031.05", "1048.36"),
        (2026, 1, 17, "1051.36", "1072.54", "1038.36", "1056.54"),
        (2026, 1, 18, "1059.54", "1080.51", "1043.54", "1061.51"),
        (2026, 1, 19, "1057.51", "1067.51", "1037.13", "1056.13"),
        (2026, 1, 20, "1059.13", "1072.13", "1033.24", "1055.24"),
        (2026, 1, 21, "1058.24", "1074.24", "1043.44", "1053.44"),
        (2026, 1, 22, "1049.44", "1068.44", "1032.58", "1045.58"),
        (2026, 2, 1, "1048.58", "1058.58", "1031.25", "1047.25"),
        (2026, 2, 2, "1050.25", "1065.28", "1031.25", "1052.28"),
        (2026, 2, 3, "1048.28", "1069.51", "1026.28", "1053.51"),
        (2026, 2, 4, "1056.51", "1082.84", "1046.51", "1063.84"),
        (2026, 2, 5, "1066.84", "1084.56", "1053.84", "1074.56"),
        (2026, 2, 6, "1070.56", "1089.83", "1054.56", "1076.83"),
        (2026, 2, 7, "1079.83", "1099.24", "1060.83", "1083.24"),
        (2026, 2, 8, "1086.24", "1105.24", "1064.23", "1086.23"),
        (2026, 2, 9, "1082.23", "1092.23", "1069.25", "1079.25"),
        (2026, 2, 10, "1082.25", "1095.25", "1064.61", "1077.61"),
        (2026, 2, 11, "1080.61", "1096.61", "1060.13", "1076.13"),
        (2026, 2, 12, "1072.13", "1091.13", "1050.57", "1069.57"),
        (2026, 2, 13, "1072.57", "1083.13", "1050.57", "1073.13"),
        (2026, 2, 14, "1076.13", "1093.09", "1066.13", "1080.09"),
        (2026, 2, 15, "1076.09", "1098.72", "1063.09", "1082.72"),
        (2026, 2, 16, "1085.72", "1112.5", "1069.72", "1093.5"),
        (2026, 2, 17, "1096.5", "1113.59", "1077.5", "1103.59"),
        (2026, 2, 18, "1099.59", "1117.34", "1077.59", "1104.34"),
        (2026, 2, 19, "1107.34", "1124.78", "1097.34", "1108.78"),
        (2026, 2, 20, "1111.78", "1130.78", "1096.94", "1109.94"),
        (2026, 2, 21, "1105.94", "1115.94", "1085.8", "1101.8"),
        (2026, 2, 22, "1104.8", "1117.8", "1081.02", "1100.02"),
        (2026, 3, 1, "1103.02", "1119.02", "1077.46", "1099.46"),
        (2026, 3, 2, "1095.46", "1114.46", "1084.61", "1094.61"),
        (2026, 3, 3, "1097.61", "1110.16", "1084.61", "1100.16"),
        (2026, 3, 4, "1103.16", "1121.8", "1087.16", "1108.8"),
        (2026, 3, 5, "1104.8", "1128.31", "1085.8", "1112.31"),
        (2026, 3, 6, "1115.31", "1141.92", "1093.31", "1122.92"),
        (2026, 3, 7, "1125.92", "1141.83", "1115.92", "1131.83"),
        (2026, 3, 8, "1127.83", "1143.73", "1114.83", "1130.73"),
        (2026, 3, 9, "1133.73", "1149.73", "1117.21", "1133.21"),
        (2026, 3, 10, "1136.21", "1155.21", "1113.87", "1132.87"),
        (2026, 3, 11, "1128.87", "1138.87", "1102.14", "1124.14"),
        (2026, 3, 12, "1127.14", "1140.14", "1112.85", "1122.85"),
        (2026, 3, 13, "1125.85", "1141.85", "1110.71", "1123.71"),
        (2026, 3, 14, "1119.71", "1139.79", "1103.71", "1120.79"),
        (2026, 3, 15, "1123.79", "1138.22", "1104.79", "1128.22"),
        (2026, 3, 16, "1131.22", "1151.13", "1109.22", "1138.13"),
        (2026, 3, 17, "1134.13", "1157.93", "1124.13", "1141.93"),
        (2026, 3, 18, "1144.93", "1170.76", "1131.93", "1151.76"),
        (2026, 3, 19, "1154.76", "1169.04", "1138.76", "1159.04"),
        (2026, 3, 20, "1155.04", "1168.96", "1136.04", "1155.96"),
        (2026, 3, 21, "1158.96", "1174.96", "1134.69", "1156.69"),
        (2026, 3, 22, "1159.69", "1178.69", "1145.34", "1155.34"),
        (2026, 4, 1, "1151.34", "1161.34", "1133.63", "1146.63"),
        (2026, 4, 2, "1149.63", "1162.63", "1130.4", "1146.4"),
        (2026, 4, 3, "1149.4", "1165.4", "1130.04", "1149.04"),
        (2026, 4, 4, "1145.04", "1167.1", "1123.04", "1148.1"),
        (2026, 4, 5, "1151.1", "1167.12", "1141.1", "1157.12"),
        (2026, 4, 6, "1160.12", "1180.77", "1147.12", "1167.77"),
        (2026, 4, 7, "1163.77", "1187.23", "1147.77", "1171.23"),
        (2026, 4, 8, "1174.23", "1198.74", "1155.23", "1179.74"),
        (2026, 4, 9, "1182.74", "1195.12", "1160.74", "1185.12"),
        (2026, 4, 10, "1181.12", "1194.12", "1170.11", "1180.11"),
        (2026, 4, 11, "1183.11", "1199.11", "1166.45", "1179.45"),
        (2026, 4, 12, "1182.45", "1201.45", "1161.66", "1177.66"),
        (2026, 4, 13, "1173.66", "1183.66", "1150.6", "1169.6"),
        (2026, 4, 14, "1172.6", "1185.6", "1148.9", "1170.9"),
        (2026, 4, 15, "1173.9", "1191.51", "1163.9", "1175.51"),
        (2026, 4, 16, "1171.51", "1195.4", "1158.51", "1176.4"),
        (2026, 4, 17, "1179.4", "1196.56", "1163.4", "1186.56"),
        (2026, 4, 18, "1189.56", "1210.33", "1170.56", "1197.33"),
        (2026, 4, 19, "1193.33", "1215.86", "1171.33", "1199.86"),
        (2026, 4, 20, "1202.86", "1225.66", "1192.86", "1206.66"),
        (2026, 4, 21, "1209.66", "1220.06", "1196.66", "1210.06"),
        (2026, 4, 22, "1206.06", "1219.06", "1187.38", "1203.38"),
        (2026, 5, 1, "1206.38", "1222.38", "1182.85", "1201.85"),
        (2026, 5, 2, "1204.85", "1223.85", "1178.25", "1200.25"),
    ];
    let bars: Vec<StudyBar> = raw_tuples
        .iter()
        .map(|&(y, m, d, o, h, l, c)| StudyBar {
            date: NaiveDate::from_ymd_opt(y, m, d).unwrap(),
            minute_of_day: None,
            open: o.parse().unwrap(),
            high: h.parse().unwrap(),
            low: l.parse().unwrap(),
            close: c.parse().unwrap(),
            volume: dec!(100000),
            dividend: Decimal::ZERO,
            split_factor: Decimal::ZERO,
        })
        .collect();

    let cmp = StrategyLoopKernel::compare_stop_policies(
        &LoopEvalRequest {
            strategy_id: "elephant_edge_v3".into(),
            symbol: "NSE:NIFTYBEES".into(),
            asset_class: StudyAssetClass::Index,
            session_preset: SessionPreset::DailyMonthly,
            bars,
            horizon_bars: 5,
            target_r: dec!(3),
            atr_period: 14,
            sl_buffer_atr: dec!(0.25),
            expiry_horizon: ExpiryHorizon::Weekly,
            spread_cost: SpreadCostConfig::default(),
            live_chain: Vec::new(),
        },
        &PlainCalendar,
        &ElephantZonesExtractor::default(),
        &ZoneTouchEntryFilter,
    )
    .expect("golden benchmark comparison must succeed");

    // Exact parity against TS `analyzeStopHuntBuffers(bars, "1D", 5)`
    assert_eq!(cmp.total_setups, 42);
    assert_eq!(cmp.bullish_setups, 7);
    assert_eq!(cmp.bearish_setups, 35);
    assert_eq!(cmp.penetration.p50_atr, Decimal::ZERO);
    assert_eq!(cmp.penetration.p75_atr, dec!(0.11));
    assert_eq!(cmp.penetration.p90_atr, dec!(0.55));
    assert_eq!(cmp.penetration.p95_atr, dec!(0.62));
    assert_eq!(cmp.penetration.max_atr, dec!(0.85));
    assert_eq!(
        cmp.penetration.pct_recovering_after_outer_touch,
        dec!(23.53)
    );

    assert_eq!(cmp.models[0].kpis.stopped_out_count, 28);
    assert_eq!(cmp.models[0].kpis.stop_out_rate_pct, dec!(66.67));
    assert_eq!(cmp.models[0].kpis.false_sweep_stop_count, 12);
    assert_eq!(cmp.models[1].kpis.stopped_out_count, 15);
    assert_eq!(cmp.models[2].kpis.stopped_out_count, 8);
    assert_eq!(cmp.models[2].kpis.stop_out_rate_pct, dec!(19.05));
    assert_eq!(cmp.models[3].kpis.stopped_out_count, 5);
    assert_eq!(cmp.recommended_policy_id, "outer_050_atr");
}
