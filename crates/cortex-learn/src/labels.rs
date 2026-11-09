//! What a task's end says of the injections its prompt got. Each is labelled with whether the task
//! used one of its first [`LABELLED_FILES`] files, edited it or read it and edited a file that
//! names it, and counted for how many of its files the task read or edited. A retrieval the gate
//! kept back and a held-out task's are labelled the same way, from what the agent did without
//! them, so that the labels are the same measure with Cortex's help and without it.
//!
//! The roles come from the task's working set before its cap: a label says what the task did,
//! whatever the links keep of it.

use std::collections::BTreeMap;

use cortex_core::{HookKind, ReasonKind};
use cortex_store::{InjectedFile, NodeId, StoreError, WriteTx};

use crate::links::{Role, file_roles};
use crate::task_state::TaskState;

/// How many of an injection's files, the best first, its label looks at.
pub const LABELLED_FILES: usize = 3;

/// Labels each injection of the task `state` shows ended, in `transaction`: with how many of its
/// files the task read or edited, and whether it used one of the first [`LABELLED_FILES`]. A task
/// with no injection writes nothing. Returns how many of the injections labelled reached the agent,
/// which is when the self-check has new labels to look at.
///
/// # Errors
///
/// The store's errors.
pub fn label_injections(
    transaction: &mut WriteTx<'_>,
    state: &TaskState,
) -> Result<usize, StoreError> {
    // Only a prompt's work order names files the task may use, and only it carries the gate's
    // features; the other texts a task is given (warnings, hints, a recap, a stop's reason, the
    // coordination notices a prompt is given) name none and are counted, never labelled.
    let rows: Vec<_> = transaction
        .queries()
        .injections_of_task(state.task_id())?
        .into_iter()
        .filter(|row| row.record.hook == HookKind::Prompt && row.record.features.is_some())
        .collect();
    if rows.is_empty() {
        return Ok(0);
    }
    let roles = file_roles(transaction.queries(), state)?;
    let mut delivered = 0;
    for row in rows {
        let (used_count, label) = labelled(&row.record.files, &roles);
        transaction.label_injection(row.id, used_count, label)?;
        delivered += usize::from(row.record.was_delivered());
    }
    Ok(delivered)
}

/// Counts, in `transaction`, the use of each line of the task `state` shows ended that named the
/// file earlier tasks read next: 1 when the task read or edited that file, which it had not when
/// the line was given, else 0. The count is all such a line gets; its label stays unset, so that it
/// never weighs in the self-check or the confidence model, which judge work orders.
///
/// # Errors
///
/// The store's errors.
pub fn count_next_file_use(
    transaction: &mut WriteTx<'_>,
    state: &TaskState,
) -> Result<(), StoreError> {
    let rows: Vec<_> = transaction
        .queries()
        .injections_of_task(state.task_id())?
        .into_iter()
        .filter(|row| {
            row.record
                .files
                .iter()
                .any(|file| file.reason == Some(ReasonKind::ReadNext))
        })
        .collect();
    if rows.is_empty() {
        return Ok(());
    }
    let roles = file_roles(transaction.queries(), state)?;
    for row in rows {
        let used = row
            .record
            .files
            .iter()
            .filter(|file| file.reason == Some(ReasonKind::ReadNext))
            .filter(|file| roles.contains_key(&file.node))
            .count();
        transaction.count_injection_use(row.id, u32::try_from(used).unwrap_or(u32::MAX))?;
    }
    Ok(())
}

/// The used count and the label of an injection of `files`, the best first, for a task whose files
/// had `roles`.
fn labelled(files: &[InjectedFile], roles: &BTreeMap<NodeId, Role>) -> (u32, bool) {
    let role_of = |file: &InjectedFile| roles.get(&file.node).copied();
    let used_count = files
        .iter()
        .filter(|file| role_of(file).is_some_and(Role::is_read_or_edited))
        .count();
    let label = files
        .iter()
        .take(LABELLED_FILES)
        .any(|file| role_of(file).is_some_and(Role::is_edited_or_read_and_used));
    (u32::try_from(used_count).unwrap_or(u32::MAX), label)
}
