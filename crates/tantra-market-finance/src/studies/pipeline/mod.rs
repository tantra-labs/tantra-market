//! Composable Strategy, Optimization-Loop & Expiry-Hedge Primitives (`Decimal`-only, `wasm32`-ready).
//!
//! Track **T21.1** (Plan 0015 Phase 1): Factors any strategy cleanly into six orthogonal,
//! reusable primitives:
//! 1. [`SessionClock`] + [`SplitArtifactGuard`] — regime-aware bar slicing & split artifact scrubbing
//! 2. [`ZoneExtractor`] (`ElephantZonesExtractor`, `Orb15Extractor`) — bands & reference equilibrium
//! 3. [`IndicatorPrimitive`] (`StandardIndicatorEvaluator`) — ATR, EMA, SMA, RSI, VWAP, VolRatio
//! 4. [`EntrySignalPrimitive`] (`ZoneTouchEntryFilter`, `RejectionWickFilter`, `OrbBreakoutEntryFilter`)
//! 5. [`StopExitPolicy`] (`OuterBufferStopPolicy`) — tight vs outer-ATR buffer & trail simulation
//! 6. [`HedgeOverlayPolicy`] (`HedgeSynthesizer`) — Day/Weekly/Monthly/Commodity expiry hedges
//!
//! Orchestrated generically via [`StrategyLoopKernel`] & [`ParameterSweepRunner`].

pub mod hedges;
pub mod hygiene;
pub mod kernel;
pub mod signals;
pub mod stops;
pub mod types;
pub mod zones;

pub use hedges::HedgeSynthesizer;
pub use hygiene::{SessionClock, SplitArtifactGuard, StandardIndicatorEvaluator};
pub use kernel::{ParameterSweepRunner, StrategyLoopKernel};
pub use signals::{OrbBreakoutEntryFilter, RejectionWickFilter, ZoneTouchEntryFilter};
pub use stops::OuterBufferStopPolicy;
pub use types::{
    DetectedSplit, EntrySignal, EntrySignalPrimitive, ExpiryHorizon, ExtractedLevelLine,
    ExtractedZoneBand, HedgeLeg, HedgeOverlayPolicy, HedgeRecommendation, HedgeStructureKind,
    HedgeSynthesisInput, IndicatorPrimitive, IndicatorSet, IndicatorSpec, LoopEvalReport,
    LoopEvalRequest, ParameterSweepReport, ParameterSweepRequest, PenetrationDistribution,
    SessionPreset, SessionSlice, SessionZoneMap, SignalDirection, SimulatedTrade, SplitCleanReport,
    StopComparisonReport, StopComparisonRow, StopExitPolicy, StopPolicyMode, StopSimInput,
    StrategyKpis, StrategyParameterBounds, StudyBar, SweepCandidateResult, ZoneExtractionOutput,
    ZoneExtractor, ZonePolarity,
};
pub use zones::{ElephantZonesExtractor, Orb15Extractor};
