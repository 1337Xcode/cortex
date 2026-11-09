//! What tasks read next. From each task's episode: the files in the order the task first read them
//! before its first change, then the first file it changed; each consecutive pair of different
//! files is a transition the task adds to, as much as its outcome teaches (the links' gain: a task
//! seen to work 1, one nobody can vouch for 0.3, one that failed, was reverted or was abandoned
//! nothing), on top of what the pair held, faded with the tasks since (`learning.link_decay`). A
//! task that adds nothing does not count.

use std::collections::BTreeMap;

use cortex_store::{EpisodeRecord, NodeId, StoreError, TransitionRow, WriteTx};

use crate::decay::decayed;
use crate::links::gain;

/// The transitions `record` walks, in its order, each pair once: the files it read first, then
/// the first file it changed, consecutive pairs of different files.
#[must_use]
pub fn transitions_of(record: &EpisodeRecord) -> Vec<(NodeId, NodeId)> {
    let walked: Vec<NodeId> = record
        .localize
        .iter()
        .map(|read| read.node)
        .chain(record.change.first().map(|changed| changed.node))
        .collect();
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

/// Adds the transitions `record`, a task's episode, walks, in `transaction`, at the task's number
/// `seq` with the decay `link_decay`, as the module says.
///
/// # Errors
///
/// The store's errors.
pub fn add_transitions(
    transaction: &mut WriteTx<'_>,
    record: &EpisodeRecord,
    (seq, link_decay): (u64, f64),
) -> Result<(), StoreError> {
    let added = gain(record.outcome);
    let pairs = transitions_of(record);
    if added <= 0.0 || pairs.is_empty() {
        return Ok(());
    }
    let sources: Vec<NodeId> = pairs.iter().map(|(from, _)| *from).collect();
    let held: BTreeMap<(NodeId, NodeId), TransitionRow> = transaction
        .queries()
        .transitions_from(&sources)?
        .into_iter()
        .map(|row| ((row.src, row.dst), row))
        .collect();
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
