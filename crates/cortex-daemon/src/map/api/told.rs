//! The times Cortex named a file to an agent, and whether the agent then used it: the work orders
//! that listed the file and the warnings that pointed to it, latest first.
//!
//! "Used" is what the store says once a task ended: a warning is resolved when its target was
//! read or edited after it; a work order's file was used when the task read or edited it.
//! Neither is known while the task runs, which the answer says with `null`.

use cortex_core::{HookKind, Ulid};
use cortex_store::{InjectionRow, NodeId, Queries, StoreError, WarningRow};
use serde::Serialize;

/// The most times the answer names: the last twenty, enough to see whether Cortex's help is
/// followed without paging through a busy file's whole history.
pub(super) const MAX_TOLD: usize = 20;

/// One time Cortex named a file to an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Told {
    /// What named it: a `work_order` or a `warning`.
    kind: &'static str,
    /// When, in UTC milliseconds.
    at_ms: i64,
    /// Whether the agent then read or edited the file, once known.
    used: Option<bool>,
}

/// The times Cortex named the file at node `file`, the latest first, at most [`MAX_TOLD`]; a work
/// order and a warning at one moment come work order first.
///
/// # Errors
///
/// SQLite errors and stored values that cannot be read back.
pub(super) fn of_file(queries: Queries<'_>, file: NodeId) -> Result<Vec<Told>, StoreError> {
    let mut told = Vec::new();
    for order in queries.work_orders_naming(file, MAX_TOLD)? {
        told.push(work_order(queries, file, &order)?);
    }
    told.extend(queries.warnings_naming(file, MAX_TOLD)?.iter().map(warning));
    told.sort_by_key(|entry| (std::cmp::Reverse(entry.at_ms), entry.kind != "work_order"));
    told.truncate(MAX_TOLD);
    Ok(told)
}

/// `order` as a time the file was named: used when its task read or edited the file, known once the
/// task's end labelled the order.
fn work_order(
    queries: Queries<'_>,
    file: NodeId,
    order: &InjectionRow,
) -> Result<Told, StoreError> {
    let task: Option<Ulid> = order.record.task_id;
    let used = match (order.used_count, task) {
        (Some(_), Some(task)) => Some(queries.has_touched(task, file, usize::MAX)?),
        _ => None,
    };
    debug_assert_eq!(order.record.hook, HookKind::Prompt);
    Ok(Told {
        kind: "work_order",
        at_ms: order.record.at_ms,
        used,
    })
}

/// `row` as a time the file was named: used when the warning was resolved.
fn warning(row: &WarningRow) -> Told {
    Told {
        kind: "warning",
        at_ms: row.record.at_ms,
        used: row.resolved,
    }
}
