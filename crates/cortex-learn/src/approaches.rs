//! The approaches a task tried that did not hold, read from what it did: the files it changed and
//! then changed back until they held what they held before it, and the checks whose same failing
//! tests stayed failing over its changes until it changed another file, after which the check
//! passed. Both are facts of the record (node ids, a runner's name, the order things happened in),
//! never a reading of what the agent meant, and nothing the agent wrote.

use std::collections::BTreeMap;

use cortex_core::{Identifier, RepoPath, ShortHash};
use cortex_store::{NodeId, PersistedFailure};

use crate::outcome::{FileNow, holds_again};
use crate::task_state::{FileNode, RunCheck, Step, TaskState};

/// The most files a persisted failure names on either side: a failure that outlasted changes to
/// more files says too little about any one of them to be told.
pub const MAX_PERSISTED_FILES: usize = 8;

/// The files `state` changed that hold, at its end, what they held before it first changed them,
/// as `now` says (each read after the task last changed it), in the order it first changed them.
/// A file the index does not hold, one whose earlier content was not seen, and every file of a
/// state that shows only part of what the task did, are left out.
#[must_use]
pub fn changed_back(state: &TaskState, now: &BTreeMap<RepoPath, FileNow>) -> Vec<NodeId> {
    if state.is_partial() {
        return Vec::new();
    }
    let mut last_ms: BTreeMap<&RepoPath, i64> = BTreeMap::new();
    let mut order: Vec<(&RepoPath, NodeId)> = Vec::new();
    for step in state.steps() {
        let Step::Edit { at_ms, file, .. } = step else {
            continue;
        };
        let Some(node) = file.node else { continue };
        last_ms.insert(&file.path, *at_ms);
        if !order.iter().any(|(path, _)| *path == &file.path) {
            order.push((&file.path, node));
        }
    }
    order
        .into_iter()
        .filter(|(path, _)| {
            last_ms.get(path).is_some_and(|edited_at_ms| {
                holds_again(state.before().get(*path), now.get(*path), *edited_at_ms)
            })
        })
        .map(|(_, node)| node)
        .collect()
}

/// A test run seen failing with the same tests, not yet seen passing.
#[derive(Debug, Clone, Copy)]
struct Streak {
    /// The signature of its failing tests.
    signature: ShortHash,
    /// The step of its latest failing run.
    last_failed: usize,
    /// Whether the task changed a file between two of its failing runs.
    has_outlasted_a_change: bool,
}

/// What tells one test run apart from another: its runner and the hash of its command as cleaned.
/// One command is one check of one kind, so a build, a type check or a lint of the same runner
/// (`cargo check` beside `cargo test`) is another run, as is another selection (`pytest -k a`).
type RunKey<'a> = (&'a Identifier, ShortHash);

/// The test runs of `state` whose same failing tests stayed failing over the task's changes until
/// it changed another file, after which the same run passed, each where it passed.
///
/// A failure persisted when one test run (one runner, one command as cleaned: `pytest -k a` is not
/// `pytest -k b`) failed naming the same tests (the same signature) at least twice with a change to
/// a file between two of those runs; it ended when the same run next passed, with a change after
/// its last failing run to a file the task had not changed before that run. A run of another kind
/// (a build, a type check, a lint) or of another command neither ends nor extends it. The files
/// changed by then are `edited`, the new ones `until`. Anything that leaves what was changed
/// unknown gives nothing: a git restore while it failed or of a file among `edited`, steps rebuilt
/// from rows (which keep no restore), a change to a file the index does not hold, a failing run
/// whose tests or command were not read, more than [`MAX_PERSISTED_FILES`] files on either side.
#[must_use]
pub fn persisted_failures(state: &TaskState) -> Vec<PersistedFailure> {
    if state.rebuilt_steps() > 0 {
        return Vec::new();
    }
    let steps = state.steps();
    let mut open: BTreeMap<RunKey<'_>, Streak> = BTreeMap::new();
    let mut found = Vec::new();
    for (at, step) in steps.iter().enumerate() {
        let Step::Run {
            check,
            exit_code,
            failing,
            restores,
            selection,
            ..
        } = step
        else {
            continue;
        };
        if !restores.is_empty() {
            open.clear();
            continue;
        }
        let (Some(check), Some(code), Some(selection)) = (check, exit_code, selection) else {
            continue;
        };
        let key = (&check.runner, *selection);
        if *code == 0 {
            let ended = open.remove(&key);
            if let Some(persisted) = ended.and_then(|streak| ended_at(steps, streak, (at, check))) {
                found.push(persisted);
            }
            continue;
        }
        let Some(signature) = failing else {
            open.remove(&key);
            continue;
        };
        let streak = match open.get(&key) {
            Some(streak) if streak.signature == *signature => Streak {
                last_failed: at,
                has_outlasted_a_change: streak.has_outlasted_a_change
                    || changed_between(steps, streak.last_failed, at),
                ..*streak
            },
            _ => Streak {
                signature: *signature,
                last_failed: at,
                has_outlasted_a_change: false,
            },
        };
        open.insert(key, streak);
    }
    found
}

/// The persisted failure `streak` was, now that its run passed at step `passed` of `steps`; none
/// when it outlasted no change, the change that ended it touched no new file, or a git restore
/// before the pass put back a file among those changed by its last failing run.
fn ended_at(
    steps: &[Step],
    streak: Streak,
    (passed, check): (usize, &RunCheck),
) -> Option<PersistedFailure> {
    if !streak.has_outlasted_a_change {
        return None;
    }
    let edited = edited_in(steps.get(..streak.last_failed)?)?;
    let is_put_back = steps.get(..passed)?.iter().any(|step| match step {
        Step::Run { restores, .. } => restores
            .iter()
            .any(|restore| edited.iter().any(|file| restore.covers(&file.path))),
        _ => false,
    });
    if is_put_back {
        return None;
    }
    let edited: Vec<NodeId> = edited.iter().filter_map(|file| file.node).collect();
    let after = edited_in(steps.get(streak.last_failed + 1..passed)?)?;
    let until: Vec<NodeId> = after
        .iter()
        .filter_map(|file| file.node)
        .filter(|node| !edited.contains(node))
        .collect();
    let fits = |nodes: &[NodeId]| !nodes.is_empty() && nodes.len() <= MAX_PERSISTED_FILES;
    (fits(&edited) && fits(&until)).then(|| PersistedFailure {
        runner: check.runner.clone(),
        edited,
        until,
    })
}

/// Whether a file was changed between steps `from` and `to` of `steps`, neither counted.
fn changed_between(steps: &[Step], from: usize, to: usize) -> bool {
    steps
        .get(from + 1..to)
        .is_some_and(|between| between.iter().any(|step| matches!(step, Step::Edit { .. })))
}

/// The files `steps` changed, each once, in the order first changed; none when one of them is a
/// file the index does not hold, which a fact could not name.
fn edited_in(steps: &[Step]) -> Option<Vec<&FileNode>> {
    let mut edited: Vec<&FileNode> = Vec::new();
    for step in steps {
        let Step::Edit { file, .. } = step else {
            continue;
        };
        file.node?;
        if !edited.iter().any(|seen| seen.path == file.path) {
            edited.push(file);
        }
    }
    Some(edited)
}
