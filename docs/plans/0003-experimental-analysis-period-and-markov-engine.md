# 0003 — Regime-Aware Period Slicer & Statistical Markov Engine (`tantra-market`)

**Status**: Phase 1 Delivered; Phase 2–3 Planned (2026-11-18) — Track **T20.2**

Master plan: root [`docs/plans/0014-experimental-analysis-statistical-workbench.md`](../../../docs/plans/0014-experimental-analysis-statistical-workbench.md).  
Depends on: [`tantra-data` plan 0010](../../../tantra-data/docs/plans/0010-experimental-analysis-multi-asset-and-events.md).

---

## 0. Executive Summary & Objective

Build a zero-float (`rust_decimal::Decimal`), `wasm32`-compatible quantitative study kernel in `tantra-market-finance` extending [`expiry.rs`](../../crates/tantra-market-finance/src/expiry.rs) (`ExpiryRules`) and [`calendar.rs`](../../crates/tantra-market-finance/src/calendar.rs) (`MarketCalendar`) to slice multi-asset OHLCV series (`F&O`, `Indices`, `ETFs`, `Equities`, `Forex`, `Commodities`) into trading cycles and compute all empirical return, **F&O spread/basis/cost-adjusted net return**, streak, Markov, distribution, outlier, intraday/ORB, correlation, multi-leg option-spread, and event-study metrics without modifying `tantra-core`.

---

## 1. Regime-Aware Period Boundary Slicer (`PeriodSchedule`)

1. **`NseWeeklyExpiry`**:
   - Instrument- and date-aware cutover table encoding SEBI/NSE changes (e.g., `NIFTY 50`: Thursday expiry `Fri→Thu` prior to `2025-09-03`, Tuesday expiry `Wed→Tue` from `2025-09-03` onward; distinct historical weekday/discontinuation rules for `BANKNIFTY`, `FINNIFTY`, `MIDCPNIFTY`, `SENSEX`).
   - Automatically shifts expiry backward to the preceding trading day via `MarketCalendar::is_trading_day` (`NseTradingCalendar`).
2. **`WeekPlusOne` & `MonthlyExpiry`**:
   - Next-cycle carry window (`cycle_k_open → cycle_{k+1}_close`) and last valid expiry day of month under the active regime.
3. **Cross-Asset Calendars**:
   - `CalendarWeekly` (`Mon→Fri` or first-to-last trading day of ISO week) & `CalendarMonthly` (for **Equities, ETFs, Global Indices**).
   - `FxSession` (daily 24h UTC/IST alignment & weekly FX sessions).
   - `McxCommodityCycle` & `FrontMonthCycle` (for **Commodities**, honoring MCX expiry schedules & continuous roll dates).

---

## 2. Pure `Decimal` Statistical Modules (`crates/tantra-market-finance/src/studies/`)

Zero `f32`/`f64` inside computation modules; square roots via deterministic Newton–Raphson `Decimal`:

1. **`returns.rs`** (**Phase 1 Delivered** + **Phase 2 F&O Spread Extension**):
   - Computes `OC` ($(C_{\text{end}} - O_{\text{start}}) / O_{\text{start}} \times 100$), `H-L` ($(\max H - \min L) / O_{\text{start}} \times 100$), and `CC` ($(C_{\text{end}} - C_{\text{prev}}) / C_{\text{prev}} \times 100$) returns + rolling SMA(`ma_window`).
   - Outputs mean, sample std dev, Sharpe, Sortino (downside semi-deviation below `0%`), Profit Factor, max gain/loss, positive/negative counts & percentages.
   - **F&O Spread, Basis & Execution Friction (`SpreadCostModel`, Phase 2)**:
     - Optional net-return transform for `F&O` / `Indices` / `Commodities` deducting round-trip **bid-ask spread** (configurable fixed `Decimal` bps / ticks or historical top-of-book `(ask - bid) / mid` per entry/exit), **futures rollover spread / calendar carry** (`F_far - F_near` on `Week+1` / `FrontMonthCycle` / `McxExpiryCycle` rolls), and statutory Indian exchange/brokerage charges via existing [`costs.rs`](../../crates/tantra-market-finance/src/india/costs.rs) (`NfoFutures`, `NfoOptions`, `CdsFutures`, `McxFutures`), reporting both **Gross Return** and **Net-After-Spread Return**.
2. **`streaks_markov.rs`** (**Phase 1 Delivered**):
   - Excludes exact `0%` returns (tracks count), builds contiguous Positive/Negative streak run-lengths & compounded returns, mode/median/std of lengths.
   - Maps streaks into 6 discrete states: `{Short 1–2, Medium 3–4, Long 5+} × {Positive, Negative}`.
   - Builds empirical `6×6` Markov transition matrix, next-streak conditional prediction from active streak state, and steady-state equilibrium $\pi$ via matrix power iteration ($\|\pi^{(t+1)} - \pi^{(t)}\|_1 < 10^{-8}$).
3. **`distributions.rs`** (**Phase 1 Delivered**):
   - Linear-interpolated empirical quantiles (`P1`, `P5`, `P25`, `P50`, `P75`, `P95`, `P99`).
   - 16 coarse return bins (`> +5%` down to `< -5%`) + non-empty `10-bps` (`0.1%`) granular bins anchored to live/latest reference price for spot price ranges.
   - `Probability-Based Levels` exceedance table (`10%, 20%, ..., 95%, 99%`).
   - `Green vs Red` conditional summary stats & PDF histograms + conditional forward-return analysis after large moves ($|r_i| \ge \tau$).
   - **Survivability Outliers**: for coverage $C \in [50, 99]$, computes symmetric lower/upper bounds (`P_{(100-C)/2}`, `P_{(100+C)/2}`), tagging every period `WithinBoundary`, `UpsideOutlier`, or `DownsideOutlier`.
4. **`intraday_orb.rs` & `correlation.rs`** (**Phase 2**):
   - Uses `5m` bars (`MarketBar`) for `IntradayMovement` (15m/1h volatility & HOD/LOD clock), `GapAnalysis` (overnight gap & gap-fill probability), `RollingNDayMove` ($k=5$), `CycleProgress0To100` (normalized intra-cycle cone), `CrossAssetCorrelation` ($N \times N$ Pearson/Spearman matrix + beta, plus **Near–Next Futures Spread / Basis Monitor**), and `Orb15FlipBacktest` (integrating [`costs.rs`](../../crates/tantra-market-finance/src/india/costs.rs) + configurable F&O bid-ask half-spread slippage on every flip leg).
5. **`options_events.rs`** (**Phase 3**):
   - **`MultiLegFnoSpreadEvaluator` & `CoveredCallSimulator` & `OptionPremiumLab`**:
     - Evaluates single-leg Covered Calls alongside **Multi-Leg F&O Spreads** (`Bull Call/Put Spread`, `Bear Call/Put Spread`, `Short Strangle / Iron Condor`, `Calendar Spread`) with strikes snapped to tick rules (`rules.rs`) from empirical `Probability-Based Levels`.
     - Models explicit **leg-by-leg bid-ask spread execution** (buy at `Ask` / sell at `Bid` or `Mid ± k × (Ask - Bid)`), margin/lot-size scaling (`lot_size`), multi-leg statutory costs (`costs.rs`), **net credit/debit after spread**, empirical **Probability of Profit (PoP)**, max gain/loss, **spread width-to-mid liquidity filter**, and **IV vs RV spread**.
   - `AnalogForecaster` (conditional `P10/P50/P90`), and `EventStudyCar` (`[-20d, +20d]` benchmark-adjusted returns for Buyback, Earnings, F&O Exclusion).

---

## 3. Verification Requirements

- Integration tests in `crates/tantra-market-finance/tests/` (keeping inline tests out of `src/`) covering pre/post Sep 3, 2025 NSE regime transitions, holiday adjustments, **F&O bid-ask spread & roll-basis net vs gross reconciliation**, **multi-leg option spread payoff & statutory fee conservation**, streak/Markov equilibrium conservation ($\sum \pi_i = 1$), quantile monotonicity, ORB-15 flip parity, and `wasm32-unknown-unknown` compilation cleanly.
