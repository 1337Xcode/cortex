//! What a task's end says of the warnings its edits got: each is resolved when the file it named
//! was read or edited later in the task, after the warning came. The share of warnings resolved
//! is their precision: a warning nobody followed up was most likely not needed.
//!
//! A read that failed touched nothing, and a step before the warning, or at its very moment (the
//! edit it answered), does not resolve it.

use cortex_store::{StoreError, WarningRow, WriteTx};

use crate::task_state::{Step, TaskState};

/// Marks each warning of the task `state` shows ended, in `transaction`, resolved or not. Returns
/// how many were marked.
///
/// # Errors
///
/// The store's errors.
pub fn resolve_warnings(
    transaction: &mut WriteTx<'_>,
    state: &TaskState,
) -> Result<usize, StoreError> {
    let rows = transaction.queries().warnings_of_task(state.task_id())?;
    for row in &rows {
        transaction.resolve_warning(row.id, is_resolved(row, state.steps()))?;
    }
    Ok(rows.len())
}

/// Whether one of `steps` read or edited the file `row` named after the warning came.
fn is_resolved(row: &WarningRow, steps: &[Step]) -> bool {
    steps.iter().any(|step| match step {
        Step::Read { at_ms, file, .. } | Step::Edit { at_ms, file, .. } => {
            file.node == Some(row.record.target) && *at_ms > row.record.at_ms
        }
        _ => false,
    })
}
