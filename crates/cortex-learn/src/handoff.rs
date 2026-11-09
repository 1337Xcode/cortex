//! The handoff a task that did not succeed leaves for the next session: written where its end is
//! learned, in place of any its session left before. A task that edited a file, or that was cut
//! off (its answer never ended: the session ended, the next prompt came first, or its turn
//! stopped on an error such as a usage limit) having read one, leaves how it ended, the files it
//! edited and read, the warnings its edits got that were not resolved, and its last check command
//! when that one failed, so that another session, of any agent, can pick the work up once this
//! one is done with it. A task that succeeds closes the open handoffs its own session left that
//! name a file it edited: the work it settled, and no other.

use cortex_core::{Identifier, TaskOutcome};
use cortex_store::{HandoffEnd, HandoffRecord, LastFailure, NodeId, StoreError, WriteTx};

use crate::task_state::{Step, TaskEnd, TaskState};

/// Leaves the handoff `state`, a task of session `session` that ended at `ended_at_ms` judged
/// `outcome`, calls for in `transaction`: one for a task that did not succeed and edited an indexed
/// file, or was cut off having read one; for a task that succeeded, its session's open handoffs
/// naming a file it edited closed. Nothing otherwise.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn leave_handoff(
    transaction: &mut WriteTx<'_>,
    state: &TaskState,
    (session, ended_at_ms, outcome): (&Identifier, i64, TaskOutcome),
) -> Result<(), StoreError> {
    if outcome == TaskOutcome::Success {
        // A success settles only the earlier work it touched: a handoff naming a file this task
        // edited is closed, one about other files stays for the next session.
        let (edited, _) = files_of(state);
        for row in transaction.queries().open_handoffs_of(session)? {
            if row.record.edited.iter().any(|node| edited.contains(node)) {
                transaction.close_handoff(row.id, session)?;
            }
        }
        return Ok(());
    }
    let end = match state.end() {
        Some(TaskEnd::Answered) => HandoffEnd::Answered,
        Some(TaskEnd::Left) => HandoffEnd::Left,
        Some(TaskEnd::Stopped) => HandoffEnd::Stopped,
        None => return Ok(()),
    };
    let (edited, read) = files_of(state);
    // A task answered that edited nothing leaves nothing to pick up; one cut off may have been
    // finding its way when it stopped.
    let is_cut_off = end != HandoffEnd::Answered;
    let has_work = !edited.is_empty() || (is_cut_off && !read.is_empty());
    if !has_work {
        return Ok(());
    }
    let open_warnings = transaction
        .queries()
        .warnings_of_task(state.task_id())?
        .into_iter()
        .filter(|warning| warning.resolved != Some(true))
        .map(|warning| warning.id)
        .collect();
    let record = HandoffRecord {
        from_session: session.clone(),
        task_id: state.task_id(),
        end,
        edited,
        read,
        open_warnings,
        last_failure: last_failure(state),
        created_at_ms: ended_at_ms,
    };
    transaction.leave_handoff(&record)?;
    Ok(())
}

/// The indexed files `state` edited, in the order it first edited them, and those it read and did
/// not edit, in the order it first read them.
fn files_of(state: &TaskState) -> (Vec<NodeId>, Vec<NodeId>) {
    let mut edited: Vec<NodeId> = Vec::new();
    let mut read: Vec<NodeId> = Vec::new();
    for step in state.steps() {
        match step {
            Step::Edit { file, .. } => {
                if let Some(node) = file.node.filter(|node| !edited.contains(node)) {
                    edited.push(node);
                }
            }
            Step::Read { file, .. } => {
                if let Some(node) = file.node.filter(|node| !read.contains(node)) {
                    read.push(node);
                }
            }
            Step::Search { .. } | Step::Run { .. } => {}
        }
    }
    read.retain(|node| !edited.contains(node));
    (edited, read)
}

/// The last check command `state` ran, when it failed: its runner, kind and exit code. None when
/// the last check passed, ran in the background, or there was none.
fn last_failure(state: &TaskState) -> Option<LastFailure> {
    let (check, exit_code) = state.steps().iter().rev().find_map(|step| match step {
        Step::Run {
            check: Some(check),
            exit_code,
            ..
        } => Some((check, *exit_code)),
        _ => None,
    })?;
    let exit_code = exit_code.filter(|code| *code != 0)?;
    Some(LastFailure {
        runner: check.runner.clone(),
        kind: check.kind,
        exit_code,
    })
}
