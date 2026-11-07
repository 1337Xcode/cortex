//! What a task did so far, read from its latest rows, and the warnings it has not followed up: those
//! whose file it did not read or edit after them, by the rule the task's end marks them resolved
//! with. The recap names them; strict mode holds a stop on those of changed signatures.
//!
//! The task's latest [`MAX_TASK_ROWS`] rows are read. When it has more, a warning's follow-up may
//! lie among those not read, so none is taken as open: what is said is only what can be told.

use cortex_core::{EventKind, RepoPath, Ulid};
use cortex_learn::{MAX_TASK_ROWS, resolves};
use cortex_store::{EventRow, NodeId, Queries, StoreError, WarningRecord};

use cortex_core::is_plain;

/// A task's latest rows and the warnings it has not followed up.
#[derive(Debug)]
pub(crate) struct SoFar {
    /// The task's latest rows, the last stored first.
    pub(crate) events: Vec<EventRow>,
    /// Its warnings whose file it did not read or edit after them, the latest first; none when the
    /// task holds more rows than were read.
    pub(crate) open: Vec<WarningRecord>,
}

/// What task `task_id` did so far, as the store `queries` reads holds it.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn so_far(queries: Queries<'_>, task_id: Ulid) -> Result<SoFar, StoreError> {
    let events = queries.latest_events_of_task(task_id, MAX_TASK_ROWS)?;
    if events.len() >= MAX_TASK_ROWS {
        return Ok(SoFar {
            events,
            open: Vec::new(),
        });
    }
    let open = queries
        .warnings_of_task(task_id)?
        .into_iter()
        .rev()
        .map(|row| row.record)
        .filter(|warning| !is_followed_up(warning, &events))
        .collect();
    Ok(SoFar { events, open })
}

/// Whether one of `events` read or edited the file `warning` named after it, and did not fail.
fn is_followed_up(warning: &WarningRecord, events: &[EventRow]) -> bool {
    events.iter().any(|row| {
        let event = &row.record;
        matches!(event.kind, EventKind::Read | EventKind::Edit)
            && event.meta.is_success != Some(false)
            && resolves(warning, event.node_id, event.ts_ms)
    })
}

/// The path of the file node `node`, when it is one and its path is plain.
pub(crate) fn plain_path(
    queries: Queries<'_>,
    node: NodeId,
) -> Result<Option<RepoPath>, StoreError> {
    Ok(queries
        .node_file(node)?
        .map(|(_, path)| path)
        .filter(|path| is_plain(path.as_str())))
}
