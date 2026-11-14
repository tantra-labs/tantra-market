#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(unsafe_code)]

//! Market data catalog, Parquet readers/writers, and report exporters for Tantra.

pub mod bhavcopy;
/// Four-quadrant open-interest buildup classification (gap 0002).
pub mod buildup;
/// Bridge from legacy `f64` wire types to canonical `tantra_core` types.
pub mod canonical;
/// Options-chain enumeration/reader (gap 0001).
pub mod chain;
pub mod dataset;
pub mod export;
/// Data import sources and loaders
pub mod import;
/// Dated instrument snapshots (E1-S3).
pub mod instruments;
/// Parquet Lakehouse v2: canonical, exact-decimal, Hive-partitioned storage.
pub mod lakehouse;
/// A writable latest-quote store layered over a [`DatasetReader`].
pub mod live_quotes;
/// Open-interest time-series snapshot stores (gap 0002).
pub mod open_interest;
/// A persistent Parquet tick store (ADR 0030).
pub mod parquet_ticks;
pub mod reader;
/// Live tick recorder daemon flushing to the lakehouse.
pub mod recorder;
pub mod ticks;

pub use buildup::{BuildupAnalyzer, BuildupClass, BuildupReader, BuildupSignal, classify_buildup};
pub use chain::InstrumentChainReader;
pub use dataset::{
    ColumnarSlice, ColumnarSliceBuilder, Dataset, DatasetBuildError, DatasetChunk, DatasetFeed,
    StreamedDatasetFeed,
};
pub use export::{
    CsvReportWriter, ExportError, JsonReportWriter, MarkdownReportWriter, ReportWriter,
};
pub use import::block_deals_projection::{
    BlockDealProjection, BlockDealsProjectionError, BlockDealsProjectionReader,
};
pub use import::corporate_actions_projection::{
    CorporateActionProjection, CorporateActionsProjectionError, CorporateActionsProjectionReader,
};
pub use import::filings_projection::{
    FilingProjection, FilingsProjectionError, FilingsProjectionReader,
};
pub use import::parquet_source::{ParquetBarSource, ParquetError};
pub use instruments::{CatalogError, CatalogInstrumentMaster, snapshot_schema};
pub use lakehouse::{Lakehouse, LakehouseCompression, LakehouseError};
pub use live_quotes::LiveQuoteStore;
pub use open_interest::{
    InMemoryOpenInterestStore, OI_DIR, OiStoreError, OpenInterestSnapshot, OpenInterestStore,
    ParquetOpenInterestStore, oi_schema, write_oi_file,
};
pub use parquet_ticks::{ParquetTickStore, TICK_DIR, TickStoreError, tick_schema, write_tick_file};
pub use ports::{ChainContract, ChainReader};
pub use reader::{DatasetReader, ReaderError};
pub use recorder::{RecorderConfig, RecorderStats, TickRecorder};
pub use ticks::InMemoryTickReader;

/// Local port traits and data types (replaces the deleted `honba-kokoro-ports`).
pub mod ports;
/// Local market-data wire types (replaces the deleted `honba-kokoro-{entities,messages}`).
pub mod wire;
