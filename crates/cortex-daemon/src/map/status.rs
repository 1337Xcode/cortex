//! What the map reads of the daemon beyond the store: its status report, the one `cortex status`
//! prints, which the server keeps (the hooks' handling times, the latest errors, what the miner,
//! the graph, upkeep and the history worker are doing).
//!
//! The server owns the map, so the map holds the server only weakly: a report asked for while the
//! daemon stops, after the server is gone, is none.

use std::fmt::Debug;

use cortex_core::ipc::StatusReport;

/// What gives the daemon's status report.
pub(crate) trait Status: Send + Sync + Debug {
    /// The daemon's status now.
    fn status_report(&self) -> StatusReport;
}
