#![deny(unsafe_code)]

//! Incremental technical indicators for the Vyapaar platform.
//!
//! Every indicator implements [`Indicator`] and consumes inputs one at a
//! time. Nothing buffers the full series; each indicator holds only the
//! state it needs, so it can run over an unbounded live stream.
//!
//! # Example
//!
//! ```
//! use vyapaar_market_indicators::{Indicator, Sma, Rsi, BollingerBands};
//!
//! let mut sma = Sma::new(20);
//! let mut rsi = Rsi::new(14);
//! let mut bb  = BollingerBands::new(20, 2.0);
//!
//! for px in [100.0, 101.0, 99.5, 102.0, 103.5] {
//!     let _ = sma.update(px);
//!     let _ = rsi.update(px);
//!     let _ = bb.update(px);
//! }
//! ```

pub mod adx;
pub mod aroon;
pub mod atr;
pub mod bollinger;
pub mod cci;
pub mod chaikin_money_flow;
pub mod cmo;
pub mod cpr;
pub mod donchian;
pub mod elder_ray;
pub mod ema;
pub mod fisher_transform;
pub mod hull_ma;
pub mod ichimoku;
pub mod indicator;
pub mod keltner_channels;
pub mod kst;
pub mod macd;
pub mod mass_index;
pub mod mfi;
pub mod obv;
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

pub use aroon::{Aroon, AroonValue};
pub use atr::Atr;
pub use bollinger::{BollingerBands, BollingerValue};
pub use cci::Cci;
pub use chaikin_money_flow::ChaikinMoneyFlow;
pub use cmo::Cmo;
pub use cpr::Cpr;
pub use donchian::{Donchian, DonchianValue};
pub use elder_ray::{ElderRay, ElderRayValue};
pub use ema::Ema;
pub use fisher_transform::FisherTransform;
pub use hull_ma::HullMa;
pub use ichimoku::{Ichimoku, IchimokuValue};
pub use indicator::Indicator;
pub use keltner_channels::{KeltnerChannels, KeltnerValue};
pub use kst::Kst;
pub use macd::{Macd, MacdValue};
pub use mass_index::MassIndex;
pub use mfi::Mfi;
pub use obv::Obv;
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

use rust_decimal::prelude::ToPrimitive;
use vyapaar_core::{Price, Quantity};

/// Reads an exact [`Price`] as the `f64` input most indicators consume. This
/// is the documented indicator-layer float boundary: indicators are numeric
/// transforms, not domain aggregates, so they compute in `f64` exactly like
/// the rest of this crate's indicator catalog.
pub(crate) fn bar_f64(price: &Price) -> f64 {
    price.value().to_f64().unwrap_or(0.0)
}

/// Reads an exact [`Quantity`] (e.g. volume) as the `f64` input a volume-aware
/// indicator consumes.
pub(crate) fn quantity_f64(qty: &Quantity) -> f64 {
    qty.value().to_f64().unwrap_or(0.0)
}
