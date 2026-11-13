//! Four-quadrant open-interest buildup classification (`tantra-market/docs/gaps/0002`).
//!
//! Classic institutional-positioning detection ("smart money footprint") compares open
//! interest *change* against price *change* over a window and classifies the move:
//!
//! | Price | OI    | Classification  |
//! |-------|-------|-----------------|
//! | Up    | Up    | Long buildup    |
//! | Down  | Up    | Short buildup   |
//! | Up    | Down  | Short covering  |
//! | Down  | Down  | Long unwinding  |
//!
//! [`classify_buildup`] is the pure classification function; [`BuildupAnalyzer`] is the
//! service that computes the deltas from an [`OpenInterestStore`](crate::OpenInterestStore)
//! and a daily-bar [`BarReader`](crate::ports::BarReader) over a window. The boundary
//! decision recorded here: classification lives in `tantra-market` (it owns the OI data),
//! and the future `oi_buildup` chat tool consumes this port rather than re-implementing it.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;
use tantra_core::InstrumentId;

use crate::open_interest::OpenInterestStore;
use crate::ports::{BarReader, BarRequest, PortResult};
use crate::wire::{BarAggregation, BarSpecification, PriceType, UnixNanos};

/// The four-quadrant buildup classification of a price/OI move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildupClass {
    /// Price up, OI up — fresh long positions.
    LongBuildup,
    /// Price down, OI up — fresh short positions.
    ShortBuildup,
    /// Price up, OI down — shorts covering.
    ShortCovering,
    /// Price down, OI down — longs unwinding.
    LongUnwinding,
}

impl fmt::Display for BuildupClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::LongBuildup => "long_buildup",
            Self::ShortBuildup => "short_buildup",
            Self::ShortCovering => "short_covering",
            Self::LongUnwinding => "long_unwinding",
        };
        f.write_str(name)
    }
}

/// Classifies a price/OI move into its four-quadrant buildup class.
///
/// A non-negative delta counts as "up" (a flat move keeps the long-side class); this is
/// the deterministic rule for the degenerate `delta == 0` edge, documented so the consumer
/// and this crate agree.
pub fn classify_buildup(price_delta: Decimal, oi_delta: Decimal) -> BuildupClass {
    let price_up = price_delta >= Decimal::ZERO;
    let oi_up = oi_delta >= Decimal::ZERO;
    match (price_up, oi_up) {
        (true, true) => BuildupClass::LongBuildup,
        (false, true) => BuildupClass::ShortBuildup,
        (true, false) => BuildupClass::ShortCovering,
        (false, false) => BuildupClass::LongUnwinding,
    }
}

/// A classified price/OI move over a window.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildupSignal {
    /// The instrument whose position was analysed.
    pub instrument: InstrumentId,
    /// Inclusive start of the window.
    pub window_start: NaiveDate,
    /// Inclusive end of the window.
    pub window_end: NaiveDate,
    /// Close-to-close price change over the window.
    pub price_delta: Decimal,
    /// Open-interest change over the window (contracts).
    pub oi_delta: Decimal,
    /// The four-quadrant class.
    pub class: BuildupClass,
}

/// Serves the classified price/OI buildup for an instrument over a window.
#[async_trait]
pub trait BuildupReader: Send + Sync {
    /// Returns the buildup signal over `[window_start, window_end]`, or `None` when the
    /// window has too few observations (fewer than two OI snapshots or daily bars) to
    /// compute a meaningful delta.
    async fn buildup(
        &self,
        instrument: &InstrumentId,
        window_start: NaiveDate,
        window_end: NaiveDate,
    ) -> PortResult<Option<BuildupSignal>>;
}

/// A [`BuildupReader`] computing deltas from an OI store and a daily-bar reader.
pub struct BuildupAnalyzer {
    oi: Arc<dyn OpenInterestStore>,
    bars: Arc<dyn BarReader>,
}

impl BuildupAnalyzer {
    /// Creates an analyzer over the given OI store and daily-bar source.
    pub fn new(oi: Arc<dyn OpenInterestStore>, bars: Arc<dyn BarReader>) -> Self {
        Self { oi, bars }
    }
}

#[async_trait]
impl BuildupReader for BuildupAnalyzer {
    async fn buildup(
        &self,
        instrument: &InstrumentId,
        window_start: NaiveDate,
        window_end: NaiveDate,
    ) -> PortResult<Option<BuildupSignal>> {
        if window_start > window_end {
            return Ok(None);
        }
        let oi_series = self
            .oi
            .history(instrument, window_start, Some(window_end))
            .await?;
        let spec = BarSpecification::new(1, BarAggregation::Day, PriceType::Last);
        let start_ns = window_start.and_hms_opt(0, 0, 0).unwrap().and_utc();
        let end_exclusive = window_end
            .checked_add_days(chrono::Days::new(1))
            .unwrap_or(window_end)
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let request = BarRequest::new(
            instrument.clone(),
            spec,
            Some(UnixNanos::from_datetime(start_ns)),
            Some(UnixNanos::from_datetime(end_exclusive)),
        )?;
        let bars = self.bars.read_bars(&request).await?;

        if oi_series.len() < 2 || bars.len() < 2 {
            return Ok(None);
        }
        let oi_delta =
            oi_series.last().unwrap().open_interest.0 - oi_series.first().unwrap().open_interest.0;
        let price_delta =
            Decimal::from_f64_retain(bars.last().unwrap().close() - bars.first().unwrap().close())
                .unwrap_or(Decimal::ZERO);

        Ok(Some(BuildupSignal {
            instrument: instrument.clone(),
            window_start,
            window_end,
            price_delta,
            oi_delta,
            class: classify_buildup(price_delta, oi_delta),
        }))
    }
}
