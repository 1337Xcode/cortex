//! What tasks read next. From each task's steps: the files in the order the task first read them
//! before its first change, then the file of that first change; each consecutive pair of different
//! files is a way the task went, which it adds to as much as its outcome teaches (the links' gain:
//! a task seen to work 1, one nobody can vouch for 0.3, one that failed, was reverted or was
//! abandoned nothing), on top of what the way held, faded with the tasks since
//! (`learning.link_decay`). A task that adds nothing does not count. A task found reverted later
//! has what it added taken back out.
//!
//! The walk keeps to what happened: past [`MAX_WALKED_READS`] files read before the first change,
//! the reads after the cap and the step to the change are left out (the file the cap stopped at was
//! not the one read before the change); and a first change to a file with no node (a file the
//! change made) ends the walk without a step to it.

use std::collections::BTreeMap;

use cortex_core::TaskOutcome;
use cortex_store::{NodeId, StoreError, TransitionRow, WriteTx};

use crate::decay::decayed;
use crate::links::gain;
use crate::task_state::{Step, TaskState};

/// The most files read before the first change that a walk follows.
pub const MAX_WALKED_READS: usize = 20;

/// The ways `state`'s task walks, in its order, each once, as the module says.
#[must_use]
pub fn transitions_of(state: &TaskState) -> Vec<(NodeId, NodeId)> {
    let mut walked: Vec<NodeId> = Vec::new();
    let mut is_capped = false;
    let mut changed: Option<Option<NodeId>> = None;
    for step in state.steps() {
        match step {
            Step::Read { file, .. } => {
                let Some(node) = file.node else { continue };
                if walked.contains(&node) {
                    continue;
                }
                if walked.len() == MAX_WALKED_READS {
                    is_capped = true;
                    continue;
                }
                walked.push(node);
            }
            Step::Edit { file, .. } => {
                changed = Some(file.node);
                break;
            }
            Step::Search { .. } | Step::Run { .. } => {}
        }
    }
    if let Some(Some(node)) = changed.filter(|_| !is_capped) {
        walked.push(node);
    }
    let mut pairs: Vec<(NodeId, NodeId)> = Vec::new();
    for step in walked.windows(2) {
        if let [from, to] = step
            && from != to
            && !pairs.contains(&(*from, *to))
        {
            pairs.push((*from, *to));
        }
    }
    pairs
}

/// Adds the ways `state`'s task walked, in `transaction`, as the task numbered `seq` whose outcome
/// is `outcome`, ways fading by `link_decay` a task, as the module says.
///
/// # Errors
///
/// The store's errors.
pub fn add_transitions(
    transaction: &mut WriteTx<'_>,
    state: &TaskState,
    (seq, outcome, link_decay): (u64, TaskOutcome, f64),
) -> Result<(), StoreError> {
    let added = gain(outcome);
    let pairs = transitions_of(state);
    if added <= 0.0 || pairs.is_empty() {
        return Ok(());
    }
    let held = held_ways(transaction, &pairs)?;
    let rows: Vec<TransitionRow> = pairs
        .iter()
        .map(|&(src, dst)| {
            let before = held.get(&(src, dst));
            let weight = before.map_or(0.0, |row| {
                decayed(row.weight, row.last_seq, seq, link_decay)
            });
            TransitionRow {
                src,
                dst,
                weight: weight + added,
                count: before.map_or(0, |row| row.count).saturating_add(1),
                last_seq: seq,
            }
        })
        .collect();
    transaction.set_transitions(&rows)
}

/// Takes back, in `transaction` at task `seq`, what `state`'s task added to its ways when it ended
/// as the task numbered `then` with outcome `outcome`: that gain, faded since as the ways have, and
/// one from each way's count; a way left counting no task is removed.
///
/// # Errors
///
/// The store's errors.
pub fn take_back_transitions(
    transaction: &mut WriteTx<'_>,
    state: &TaskState,
    (then, outcome, seq, link_decay): (u64, TaskOutcome, u64, f64),
) -> Result<(), StoreError> {
    let added = gain(outcome);
    let pairs = transitions_of(state);
    if added <= 0.0 || pairs.is_empty() {
        return Ok(());
    }
    let held = held_ways(transaction, &pairs)?;
    let mut kept = Vec::new();
    let mut gone = Vec::new();
    for row in pairs.iter().filter_map(|pair| held.get(pair)) {
        let count = row.count.saturating_sub(1);
        if count == 0 {
            gone.push((row.src, row.dst));
            continue;
        }
        let weight = decayed(row.weight, row.last_seq, seq, link_decay)
            - decayed(added, then, seq, link_decay);
        kept.push(TransitionRow {
            weight: weight.max(0.0),
            count,
            last_seq: seq,
            ..*row
        });
    }
    transaction.remove_transitions(&gone)?;
    transaction.set_transitions(&kept)
}

/// The ways of `pairs` the store holds, by pair.
fn held_ways(
    transaction: &WriteTx<'_>,
    pairs: &[(NodeId, NodeId)],
) -> Result<BTreeMap<(NodeId, NodeId), TransitionRow>, StoreError> {
    let sources: Vec<NodeId> = pairs.iter().map(|(from, _)| *from).collect();
    Ok(transaction
        .queries()
        .transitions_from(&sources)?
        .into_iter()
        .map(|row| ((row.src, row.dst), row))
        .collect())
}
