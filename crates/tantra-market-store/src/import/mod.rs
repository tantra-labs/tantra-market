//! Data import sources and loaders.

pub mod block_deals_projection;
pub mod corporate_actions_projection;
pub mod filings_projection;
pub mod news_projection;
pub mod parquet_source;

pub use block_deals_projection::{
    BlockDealProjection, BlockDealsProjectionError, BlockDealsProjectionReader,
};
pub use corporate_actions_projection::{
    CorporateActionProjection, CorporateActionsProjectionError, CorporateActionsProjectionReader,
};
pub use filings_projection::{FilingProjection, FilingsProjectionError, FilingsProjectionReader};
pub use news_projection::{NewsProjection, NewsProjectionError, NewsProjectionReader};
