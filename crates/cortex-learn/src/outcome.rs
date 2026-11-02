//! What became of a task: what the checks after its last edit vouch for, whether its edits were
//! put back, and whether the agent reached the end of an answer about it.
//!
//! Nothing here reads a file or the store. A judgement is made from what the task's state shows
//! and from what the caller says each file the task edited holds at present, so the same task
//! always gets the same judgement in the same words. The rules are conservative: a task counts as
//! a success only when a check that can vouch for its work was seen to pass after its last edit
//! and nothing put that work back afterwards, and as reverted only when every file it edited is
//! known to be back as it was.

use std::collections::BTreeMap;

use cortex_core::{CommandKind, FileContent, RepoPath, TaskOutcome};

use crate::commands::Restore;
use crate::task_state::{Step, TaskEnd, TaskState};

/// What a file holds at present, and when that was read.
///
/// The time matters: what a file held before a task is taken from the store when the index has
/// not yet read the change, so a store row read again before the index catches up holds that same
/// content and would pass for the file being back as it was. Only a content read after the task
/// last edited the file can show that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileNow {
    /// What it holds.
    pub content: FileContent,
    /// When that was read, in UTC milliseconds: when the file itself was read, or when the index
    /// wrote the row it was read from. A file the store has no row for and that was not read has
    /// nothing to pass here and is left out.
    pub read_at_ms: i64,
}

/// What became of a task, and the words that say why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgement {
    /// What became of it.
    pub outcome: TaskOutcome,
    /// Why, in words that hold no command line, path or code: what a check vouched for, the name
    /// of the runner that ran it and its exit code, or what put the task's files back. `None`
    /// when nothing is known.
    pub evidence: Option<String>,
}

/// What became of the task `state`, which has ended, with `now` giving what each file it edited
/// holds at present: an entry for every path of [`TaskState::before`], each read after the task
/// last edited that file.
///
/// The rules in the order they are tried: a state that shows only part of what the task did says
/// nothing; a task whose every edited file is back as it was was reverted; otherwise the last
/// check that finished after the last edit tells, unless a git restore put the task's work back
/// after it; otherwise a task left before the end of an answer was abandoned. Nothing else is
/// known.
#[must_use]
pub fn judge(state: &TaskState, now: &BTreeMap<RepoPath, FileNow>) -> Judgement {
    if state.is_partial() {
        return unknown();
    }
    let did = Did::of(state);
    if let Some(evidence) = put_back(state, &did, None, now) {
        return reverted(evidence);
    }
    if let Some(judgement) = checked(state, &did) {
        return judgement;
    }
    if state.end() == Some(TaskEnd::Left) {
        return Judgement {
            outcome: TaskOutcome::Abandoned,
            evidence: Some("the task was left before its answer ended".to_owned()),
        };
    }
    unknown()
}

/// That the task `previous` was reverted, as the end of `next`, the task after it, shows: every
/// file `previous` edited is back as it was before it, by what the files hold now or by a git
/// restore in either task after `previous` edited that file. `None` when its work stands, or when
/// its state shows only part of what it did.
///
/// `now` is as for [`judge`], read when `next` ended. The caller pairs a task with the task after
/// it; a restore of `next` counts only when it ran after the edit it undoes, so a pair that is not
/// in that order shows no revert it did not see.
#[must_use]
pub fn reverted_after(
    previous: &TaskState,
    next: &TaskState,
    now: &BTreeMap<RepoPath, FileNow>,
) -> Option<Judgement> {
    let (did, after) = (Did::of(previous), Did::of(next));
    put_back(previous, &did, Some(&after), now).map(reverted)
}

/// A judgement that says nothing.
fn unknown() -> Judgement {
    Judgement {
        outcome: TaskOutcome::Unknown,
        evidence: None,
    }
}

/// A task reverted for the reason `evidence` gives.
fn reverted(evidence: String) -> Judgement {
    Judgement {
        outcome: TaskOutcome::Reverted,
        evidence: Some(evidence),
    }
}

/// When a task last edited a file.
#[derive(Debug, Clone, Copy)]
struct LastEdit {
    /// Which of the task's steps did it. Steps of one task are ordered where their times are not:
    /// two can share a millisecond.
    step: usize,
    /// When, in UTC milliseconds, which orders it against another task's steps.
    at_ms: i64,
}

/// A git restore a task ran.
struct RestoreStep<'a> {
    /// Which of the task's steps ran it.
    step: usize,
    /// When, in UTC milliseconds.
    at_ms: i64,
    /// What it put back.
    restore: &'a Restore,
}

/// What the rules read from a task's state.
struct Did<'a> {
    /// When the task last edited each file it edited, by path.
    edits: BTreeMap<RepoPath, LastEdit>,
    /// The git restores it ran, in the order it ran them.
    restores: Vec<RestoreStep<'a>>,
}

impl<'a> Did<'a> {
    /// What `state` shows the task did.
    fn of(state: &'a TaskState) -> Self {
        let mut edits = BTreeMap::new();
        let mut restores = Vec::new();
        for (step, did) in state.steps().iter().enumerate() {
            match did {
                Step::Edit { at_ms, file, .. } => {
                    edits.insert(
                        file.path.clone(),
                        LastEdit {
                            step,
                            at_ms: *at_ms,
                        },
                    );
                }
                Step::Run {
                    at_ms,
                    restores: ran,
                    ..
                } => restores.extend(ran.iter().map(|restore| RestoreStep {
                    step,
                    at_ms: *at_ms,
                    restore,
                })),
                Step::Read { .. } | Step::Search { .. } => {}
            }
        }
        Self { edits, restores }
    }
}

/// Why every file `state` edited is back as it was before it: each holds again what it held, as
/// `now` says, or a git restore put it back after the task last edited it, in `state` or in
/// `next`, the task after it. `None` unless the task edited something and every file it edited is
/// back, and for a state that shows only part of what it did, whose rows may hold the edit that
/// stands.
fn put_back(
    state: &TaskState,
    did: &Did<'_>,
    next: Option<&Did<'_>>,
    now: &BTreeMap<RepoPath, FileNow>,
) -> Option<String> {
    if state.is_partial() || did.edits.is_empty() {
        return None;
    }
    let mut by_contents = true;
    for (path, edit) in &did.edits {
        if holds_again(state.before().get(path), now.get(path), edit.at_ms) {
            continue;
        }
        by_contents = false;
        if !restored(path, *edit, (did, next)) {
            return None;
        }
    }
    let how = if by_contents {
        "is back as it was before the task"
    } else {
        "was put back or restored by git"
    };
    Some(match next {
        Some(_) => format!("every edited file {how}, by the end of the next task"),
        None => format!("every edited file {how}"),
    })
}

/// Whether a file holds again what it held, from what it held `before` a task and what it holds
/// `now`: contents of the same hash, or no file either time, read after the task edited it at
/// `edited_at_ms`. Contents the index does not read, a file neither is known for, and a content
/// read no later than the edit all say nothing.
fn holds_again(before: Option<&FileContent>, now: Option<&FileNow>, edited_at_ms: i64) -> bool {
    let Some(now) = now.filter(|now| now.read_at_ms > edited_at_ms) else {
        return false;
    };
    match (before, now.content) {
        (Some(FileContent::Hashed(before)), FileContent::Hashed(now)) => *before == now,
        (Some(FileContent::Absent), FileContent::Absent) => true,
        _ => false,
    }
}

/// Whether a git restore put back the file at `path`, last edited by the task as `edit` says: one
/// the task itself ran at a later step, or one the task after it ran at a later moment. Steps of
/// one task are ordered where their times are not; two tasks' steps have only their times.
fn restored(path: &RepoPath, edit: LastEdit, (did, next): (&Did<'_>, Option<&Did<'_>>)) -> bool {
    let own = did.restores.iter().filter(|ran| ran.step > edit.step);
    let after = next
        .into_iter()
        .flat_map(|next| next.restores.iter())
        .filter(|ran| ran.at_ms > edit.at_ms);
    own.chain(after).any(|ran| covers(ran.restore, path))
}

/// Whether `restore` put back the file at `path`: every file, the file itself, or a folder holding
/// it.
fn covers(restore: &Restore, path: &RepoPath) -> bool {
    match restore {
        Restore::Everything => true,
        Restore::Paths(paths) => std::iter::successors(Some(path.clone()), RepoPath::parent)
            .any(|place| paths.contains(&place)),
    }
}

/// What the last check that finished after the task's last edit vouches for: one that exited 0
/// says the work holds up, and one that exited anything else that it does not. `None` when the
/// task edited nothing, or when no such check finished after its last edit with its work still in
/// place: a check before the last edit says nothing about what the task left behind, one a git
/// restore of an edited file followed vouches for work that is gone, a lint vouches for no work,
/// and a command that was interrupted or runs on in the background finished nothing.
fn checked(state: &TaskState, did: &Did<'_>) -> Option<Judgement> {
    let steps = state.steps();
    let last_edit = steps
        .iter()
        .rposition(|step| matches!(step, Step::Edit { .. }))?;
    let vouches_from = undone_at(did, last_edit).unwrap_or(last_edit);
    let (vouched, runner, code) = steps
        .get(vouches_from + 1..)?
        .iter()
        .rev()
        .find_map(|step| match step {
            Step::Run {
                check: Some(check),
                exit_code: Some(code),
                ..
            } => Some((vouched_for(check.kind)?, check.runner.as_str(), *code)),
            _ => None,
        })?;
    Some(Judgement {
        outcome: if code == 0 {
            TaskOutcome::Success
        } else {
            TaskOutcome::Failure
        },
        // INVARIANT: no-content-stored (the words name what the check vouched for, the runner
        // Cortex knows it by and its exit code, never the command line)
        evidence: Some(format!(
            "{vouched} by {runner} exited {code} after the last edit"
        )),
    })
}

/// The last step after `last_edit` at which the task put back a file it had edited, whose restore
/// leaves every check up to and including that step vouching for work that is gone. The step
/// itself counts, because a line that both checks and restores (`cargo test && git restore a.py`)
/// is one step whose two halves are not ordered in it.
fn undone_at(did: &Did<'_>, last_edit: usize) -> Option<usize> {
    did.restores
        .iter()
        .filter(|ran| ran.step > last_edit)
        .filter(|ran| did.edits.keys().any(|path| covers(ran.restore, path)))
        .map(|ran| ran.step)
        .max()
}

/// What a check of `kind` vouches for, in words: `None` for one that vouches for no work, as a
/// lint does, which says nothing about whether a change holds up.
fn vouched_for(kind: CommandKind) -> Option<&'static str> {
    match kind {
        CommandKind::Test => Some("tests"),
        CommandKind::Build => Some("the build"),
        CommandKind::TypeCheck => Some("the type check"),
        CommandKind::Lint => None,
    }
}
