//! The handoff a task left unfinished leaves for the next session: written where its end is
//! learned. A task whose answer never ended (the session ended, a turn stopped on an API error such
//! as a usage limit, or the next prompt came first) and that did not succeed leaves the files it
//! edited and read, the warnings its edits got that were not resolved, and its last check command
//! when that one failed, so that another session, of any agent, can pick the work up. A task that
//! succeeds closes the open handoff its own session left: the work was finished there.

use cortex_core::{Identifier, TaskOutcome};
use cortex_store::{HandoffRecord, LastFailure, NodeId, StoreError, WriteTx};

use crate::task_state::{Step, TaskEnd, TaskState};

/// Leaves the handoff `state`, a task of session `session` that ended at `ended_at_ms` judged
/// `outcome`, calls for in `transaction`: one for a task left unfinished that did not succeed and
/// touched an indexed file; for a task that succeeded, its session's open handoff closed. Nothing
/// otherwise.
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
        return transaction.close_handoff(session, session);
    }
    if state.end() != Some(TaskEnd::Left) {
        return Ok(());
    }
    let (edited, read) = files_of(state);
    if edited.is_empty() && read.is_empty() {
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
