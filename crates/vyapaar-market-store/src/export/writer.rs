//! The `ReportWriter` trait.

use vyapaar_agent_analytics::PerformanceReport;

use crate::export::error::Result;

/// Something that serializes a [`PerformanceReport`].
pub trait ReportWriter {
    /// Writes the report.
    fn write(&mut self, report: &PerformanceReport) -> Result<()>;
}
