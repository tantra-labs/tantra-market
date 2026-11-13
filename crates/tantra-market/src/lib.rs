//! Facade of the `tantra-market` bounded context (Eye).
//!
//! Re-exports the member crates under their short names so consumers import the
//! whole market-data context from a single crate (facade-crate convention, root
//! plan 0003 Phase 4 / gap 0007).

pub use tantra_market_core as core;
pub use tantra_market_finance as finance;
pub use tantra_market_indicators as indicators;
pub use tantra_market_normalizer as normalizer;
pub use tantra_market_pricing as pricing;
pub use tantra_market_signals as signals;
pub use tantra_market_store as store;
