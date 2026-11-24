#![deny(unsafe_code)]

//! Incremental technical indicators for the Tantra platform.
//!
//! Every indicator implements [`Indicator`] and consumes inputs one at a
//! time in exact `Decimal` arithmetic. Nothing buffers the full series; each
//! indicator holds only the state it needs, so it can run over an unbounded
//! live stream.
//!
//! # Example
//!
//! ```
//! use rust_decimal::Decimal;
//! use tantra_market_indicators::{BollingerBands, Indicator, Rsi, Sma};
//!
//! let mut sma = Sma::new(20);
//! let mut rsi = Rsi::new(14);
//! let mut bb  = BollingerBands::new(20, Decimal::TWO);
//!
//! for px in [100, 101, 99, 102, 104] {
//!     let d = Decimal::from(px);
//!     let _ = sma.update(d);
//!     let _ = rsi.update(d);
//!     let _ = bb.update(d);
//! }
//! ```

pub mod adr_pct;
pub mod adx;
pub mod aroon;
pub mod atr;
pub mod bollinger;
pub mod cci;
pub mod chaikin_money_flow;
pub mod chandelier;
pub mod cmo;
pub mod cpr;
pub mod donchian;
pub mod elder_ray;
pub mod elephant_zones;
pub mod ema;
pub mod fisher_transform;
pub mod gamma_profile;
pub mod hull_ma;
pub mod ichimoku;
pub mod indicator;
pub mod keltner_channels;
pub mod kst;
pub mod macd;
pub mod mass_index;
pub mod mfi;
pub mod obv;
pub mod oi_levels;
pub mod order_block;
pub mod parabolic_sar;
pub mod pivot_points;
pub mod render;
pub mod rma;
pub mod roc;
pub mod rsi;
pub mod screener;
pub mod sma;
pub mod stochastic;
pub mod supertrend;
pub mod trix;
pub mod ultimate_oscillator;
pub mod vortex;
pub mod vwap;
pub mod williams_r;

pub use adr_pct::AdrPct;
pub use adx::Adx;
pub use aroon::{Aroon, AroonValue};
pub use atr::Atr;
pub use bollinger::{BollingerBands, BollingerValue};
pub use cci::Cci;
pub use chaikin_money_flow::ChaikinMoneyFlow;
pub use chandelier::{ChandelierExit, ChandelierExitValue};
pub use cmo::Cmo;
pub use cpr::Cpr;
pub use donchian::{Donchian, DonchianValue};
pub use elder_ray::{ElderRay, ElderRayValue};
pub use elephant_zones::{ElephantMode, ElephantZones, ElephantZonesValue};
pub use ema::Ema;
pub use fisher_transform::FisherTransform;
pub use gamma_profile::{GammaInput, GammaProfile, GammaProfileValue, StrikeGammaInput};
pub use hull_ma::HullMa;
pub use ichimoku::{Ichimoku, IchimokuValue};
pub use indicator::Indicator;
pub use keltner_channels::{KeltnerChannels, KeltnerValue};
pub use kst::Kst;
pub use macd::{Macd, MacdValue};
pub use mass_index::MassIndex;
pub use mfi::Mfi;
pub use obv::Obv;
pub use oi_levels::{OiLevels, OiLevelsInput, OiLevelsValue, OiWall, StrikeOi};
pub use order_block::{OrderBlock, OrderBlockValue};
pub use parabolic_sar::ParabolicSar;
pub use pivot_points::{PivotPoints, PivotStyle, PivotValue};
pub use rma::Rma;
pub use roc::Roc;
pub use rsi::Rsi;
pub use sma::Sma;
pub use stochastic::{Stochastic, StochasticValue};
pub use supertrend::Supertrend;
pub use trix::Trix;
pub use ultimate_oscillator::UltimateOscillator;
pub use vortex::{Vortex, VortexValue};
pub use vwap::Vwap;
pub use williams_r::WilliamsR;
