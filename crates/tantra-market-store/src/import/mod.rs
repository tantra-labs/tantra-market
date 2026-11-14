//! Data import sources and loaders.

pub mod block_deals_projection;
pub mod corporate_actions_projection;
pub mod delivery_projection;
pub mod economic_calendar_projection;
pub mod filings_projection;
pub mod institutional_flows_projection;
pub mod news_projection;
pub mod parquet_source;

pub use block_deals_projection::{
    BlockDealProjection, BlockDealsProjectionError, BlockDealsProjectionReader,
};
pub use corporate_actions_projection::{
    CorporateActionProjection, CorporateActionsProjectionError, CorporateActionsProjectionReader,
};
pub use delivery_projection::{
    DeliveryProjection, DeliveryProjectionError, DeliveryProjectionReader,
};
pub use economic_calendar_projection::{
    EconomicCalendarProjection, EconomicCalendarProjectionError, EconomicCalendarProjectionReader,
};
pub use filings_projection::{FilingProjection, FilingsProjectionError, FilingsProjectionReader};
pub use institutional_flows_projection::{
    InstitutionalFlowProjection, InstitutionalFlowsProjectionError,
    InstitutionalFlowsProjectionReader,
};
pub use news_projection::{NewsProjection, NewsProjectionError, NewsProjectionReader};
