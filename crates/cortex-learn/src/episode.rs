//! One record of what a task did, for the archetypes that cluster tasks later and the retrieval
//! that seeds a similar task from them.
//!
//! The record keeps where the task looked before it changed anything, what it changed, what it ran
//! to check the change, what went wrong on the way, and what became of it. It holds node ids,
//! counts, line numbers, exit codes and the names Cortex knows runners by, so a record can be read
//! without reading the work: no prompt, no command line, no code.
//!
//! What a task did can be large, and the store refuses a record past its bound, so the parts are
//! trimmed until they fit: the pitfalls first, then the checks, then what changed, and the places
//! it looked last of all.

use std::collections::BTreeMap;

use cortex_core::{Identifier, LineRange, RepoPath, TaskOutcome};
use cortex_store::{
    ChangedFile, ChangedSymbol, EpisodeRecord, MAX_EPISODE_BYTES, NodeId, Pitfalls, Queries,
    RanCheck, ReadAgain, ReadFile, StoreError,
};

use crate::approaches::{changed_back, persisted_failures};
use crate::outcome::FileNow;
use crate::places::Places;
use crate::task_state::{FileNode, Step, TaskState};

/// Most files a record keeps of where the task looked before it changed anything.
const MAX_LOCALIZE: usize = 20;

/// Most files a record keeps of what the task changed, before it is trimmed to fit.
const MAX_CHANGED: usize = 64;

/// Most checks a record keeps, before it is trimmed to fit.
const MAX_VERIFY: usize = 64;

/// Most of each kind of pitfall a record keeps, before it is trimmed to fit.
const MAX_PITFALLS: usize = 32;

/// How many times a stretch has to be read before reading it again counts as a pitfall.
const READS_BEFORE_A_PITFALL: u32 = 3;

/// The record of the task `state` describes, which ended as `outcome` says, written at
/// `created_at_ms`, read with `queries` for the symbols its changes covered, with `now` saying
/// what each file it edited holds at its end, as for [`crate::judge`].
///
/// # Errors
///
/// The store's errors.
pub fn episode_of(
    queries: Queries<'_>,
    state: &TaskState,
    now: &BTreeMap<RepoPath, FileNow>,
    (outcome, created_at_ms): (TaskOutcome, i64),
) -> Result<EpisodeRecord, StoreError> {
    let mut places = Places::new(queries);
    let mut record = EpisodeRecord {
        task_id: state.task_id(),
        localize: localize(state),
        change: change(&mut places, state)?,
        verify: verify(state),
        pitfalls: pitfalls(state, now),
        outcome,
        created_at_ms,
    };
    trim(&mut record)?;
    Ok(record)
}

/// The files the task read before it changed anything, in the order it first read each, with how
/// many times it read each of them by then.
fn localize(state: &TaskState) -> Vec<ReadFile> {
    let mut reads: Vec<ReadFile> = Vec::new();
    for step in state.steps() {
        match step {
            Step::Edit { .. } => break,
            Step::Read { file, .. } => {
                let Some(node) = file.node else { continue };
                match reads.iter_mut().find(|read| read.node == node) {
                    Some(read) => read.reads = read.reads.saturating_add(1),
                    None => reads.push(ReadFile { node, reads: 1 }),
                }
            }
            Step::Search { .. } | Step::Run { .. } => {}
        }
    }
    reads.truncate(MAX_LOCALIZE);
    reads
}

/// What the task changed, in the order it first changed each file: how many of the file's lines its
/// changes covered, and the symbols among them, each with whether a change covered the line its
/// declaration starts on. A change with no lines of its own, as a whole file written at once has,
/// covers no lines and no symbols.
fn change(places: &mut Places<'_>, state: &TaskState) -> Result<Vec<ChangedFile>, StoreError> {
    let mut order: Vec<(RepoPath, NodeId)> = Vec::new();
    let mut ranges: BTreeMap<RepoPath, Vec<LineRange>> = BTreeMap::new();
    for step in state.steps() {
        let Step::Edit { file, range, .. } = step else {
            continue;
        };
        let Some(node) = file.node else { continue };
        if !order.iter().any(|(path, _)| *path == file.path) {
            order.push((file.path.clone(), node));
        }
        if let Some(range) = range {
            ranges.entry(file.path.clone()).or_default().push(*range);
        }
    }
    order.truncate(MAX_CHANGED);
    let mut changed = Vec::new();
    for (path, node) in order {
        let ranges = ranges.get(&path).map_or(&[][..], Vec::as_slice);
        let mut symbols: BTreeMap<NodeId, bool> = BTreeMap::new();
        for range in ranges {
            for span in places.covering_spans(&path, *range)? {
                symbols.insert(span.node, covers_line(ranges, span.start_line));
            }
        }
        changed.push(ChangedFile {
            node,
            lines: lines_covered(ranges),
            symbols: symbols
                .into_iter()
                .map(|(node, header)| ChangedSymbol { node, header })
                .collect(),
        });
    }
    Ok(changed)
}

/// The checks the task ran, in order, each with its runner and how it ended.
fn verify(state: &TaskState) -> Vec<RanCheck> {
    let mut checks: Vec<RanCheck> = state
        .steps()
        .iter()
        .filter_map(|step| match step {
            Step::Run {
                check: Some(check),
                exit_code,
                ..
            } => Some(RanCheck {
                runner: check.runner.clone(),
                kind: check.kind,
                code: *exit_code,
            }),
            _ => None,
        })
        .collect();
    checks.truncate(MAX_VERIFY);
    checks
}

/// What went wrong on the task's way: a runner whose check failed and later passed, the files it
/// changed and then put back itself (with a git restore, then those `now` shows changed back), the
/// stretches it read more times than a reader needs to, and the checks whose failure stayed the
/// same until it changed another file.
fn pitfalls(state: &TaskState, now: &BTreeMap<RepoPath, FileNow>) -> Pitfalls {
    let mut undone = undone(state);
    for node in changed_back(state, now) {
        if !undone.contains(&node) {
            undone.push(node);
        }
    }
    let mut pitfalls = Pitfalls {
        fixed: fixed(state),
        undone,
        read_again: read_again(state),
        persisted: persisted_failures(state),
    };
    pitfalls.fixed.truncate(MAX_PITFALLS);
    pitfalls.undone.truncate(MAX_PITFALLS);
    pitfalls.read_again.truncate(MAX_PITFALLS);
    pitfalls.persisted.truncate(MAX_PITFALLS);
    pitfalls
}

/// The runners whose check failed and later passed, each once, in the order they first failed.
fn fixed(state: &TaskState) -> Vec<Identifier> {
    let mut failed: Vec<Identifier> = Vec::new();
    let mut fixed: Vec<Identifier> = Vec::new();
    for step in state.steps() {
        let Step::Run {
            check: Some(check),
            exit_code: Some(code),
            ..
        } = step
        else {
            continue;
        };
        if *code == 0 {
            if failed.contains(&check.runner) && !fixed.contains(&check.runner) {
                fixed.push(check.runner.clone());
            }
        } else if !failed.contains(&check.runner) {
            failed.push(check.runner.clone());
        }
    }
    fixed
}

/// The files the task changed and then put back itself with a git restore after its last change
/// of them, in the order it first changed them: a file changed again after its restore ends the
/// task changed, and is not put back.
fn undone(state: &TaskState) -> Vec<NodeId> {
    let steps = state.steps();
    let mut order: Vec<(&FileNode, usize)> = Vec::new();
    for (at, step) in steps.iter().enumerate() {
        let Step::Edit { file, .. } = step else {
            continue;
        };
        match order.iter_mut().find(|(seen, _)| seen.path == file.path) {
            Some((_, last)) => *last = at,
            None => order.push((file, at)),
        }
    }
    order
        .into_iter()
        .filter(|(file, last)| {
            steps.iter().skip(last + 1).any(|later| match later {
                Step::Run { restores, .. } => {
                    restores.iter().any(|restore| restore.covers(&file.path))
                }
                _ => false,
            })
        })
        .filter_map(|(file, _)| file.node)
        .collect()
}

/// The stretches the task read more than [`READS_BEFORE_A_PITFALL`] times, in the order it first
/// read each.
fn read_again(state: &TaskState) -> Vec<ReadAgain> {
    let mut reads: Vec<ReadAgain> = Vec::new();
    for step in state.steps() {
        let Step::Read {
            file: FileNode {
                node: Some(node), ..
            },
            range: Some(range),
            ..
        } = step
        else {
            continue;
        };
        let seen = reads.iter_mut().find(|read| {
            read.node == *node
                && read.from_line == range.start_line()
                && read.to_line == range.end_line()
        });
        match seen {
            Some(read) => read.times = read.times.saturating_add(1),
            None => reads.push(ReadAgain {
                node: *node,
                from_line: range.start_line(),
                to_line: range.end_line(),
                times: 1,
            }),
        }
    }
    reads.retain(|read| read.times > READS_BEFORE_A_PITFALL);
    reads
}

/// How many of a file's lines the `ranges` cover together, a line counted once however many of
/// them cover it.
fn lines_covered(ranges: &[LineRange]) -> u32 {
    let mut sorted: Vec<(u32, u32)> = ranges
        .iter()
        .map(|range| (range.start_line(), range.end_line()))
        .collect();
    sorted.sort_unstable();
    let mut covered: u32 = 0;
    let mut reached: u32 = 0;
    for (from, to) in sorted {
        let from = from.max(reached.saturating_add(1));
        if from > to {
            continue;
        }
        covered += to - from + 1;
        reached = to;
    }
    covered
}

/// Whether any of `ranges` covers `line`.
fn covers_line(ranges: &[LineRange], line: u32) -> bool {
    ranges
        .iter()
        .any(|range| range.start_line() <= line && line <= range.end_line())
}

/// Drops what the record can do without until its parts fit the bound the store keeps: the
/// pitfalls first, then the checks, then what changed, and the places the task looked last of all.
/// The outcome and the task are never dropped.
fn trim(record: &mut EpisodeRecord) -> Result<(), StoreError> {
    // INVARIANT: budgeted (an episode's parts hold at most MAX_EPISODE_BYTES)
    while record.size_bytes()? > MAX_EPISODE_BYTES && drop_one(record) {}
    Ok(())
}

/// Drops the last of the least of what the record holds, and says whether anything was left to
/// drop.
fn drop_one(record: &mut EpisodeRecord) -> bool {
    record.pitfalls.read_again.pop().is_some()
        || record.pitfalls.fixed.pop().is_some()
        || record.pitfalls.persisted.pop().is_some()
        || record.pitfalls.undone.pop().is_some()
        || record.verify.pop().is_some()
        || record.change.pop().is_some()
        || record.localize.pop().is_some()
}
