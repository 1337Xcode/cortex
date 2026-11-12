//! The experience hint: at most one line a work order carries about earlier work like the prompt's,
//! made of recorded facts only, while `history.experience` is on.
//!
//! ```text
//! [cortex] A similar task (2 weeks ago) changed src/orders/service.py and tests/test_orders.py;
//! editing src/orders/service.py alone left the same pytest tests failing until
//! src/orders/validate.py changed too.
//! ```
//!
//! Earlier work is a past task's record or a past change's card, and nothing new is searched to
//! find it. A task's record is like the prompt when at least half of the files it changed are among
//! the files the similar-task seeds gave: the files of the clusters of past tasks whose words match
//! the prompt's at [`crate::SIMILAR_BAR`], each cluster keeping its 50 most-changed files. Half is
//! [`JOINS_A_CLUSTER`], the clustering's Jaccard bar, reused here as a share of one task's files
//! found in that union of clusters' files: it was not measured for this use, and a task of a
//! matching cluster can fall below it when its files are not among the cluster's 50. A change's
//! card is like the prompt when it scores [`SIMILAR_CHANGE_BAR`] as the history seeds score it.
//! Records come first, the newest first, then cards, the best first; the first that can be said is
//! said. Only what each record changed is read to choose; the rest of a record is read only for one
//! that is like the prompt.
//!
//! Only work that ended strictly before the task asking began is read: a record written at that
//! moment or later, or a change committed in that second or later (a commit's time is kept to the
//! second), never reaches the hint, however like the prompt. Only a task whose work stood (one that
//! succeeded, or that nothing says failed) is told. A search the request's deadline overtakes gives
//! no hint at all, so that what the hint says never depends on how fast the machine was.
//!
//! What the hint says comes from the record or the card and from the files as they are now (see
//! `wording`): the files the work changed and kept, and for a task one approach that did not hold,
//! a failure that stayed the same until another file changed, else the files it put back.

mod wording;

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use cortex_core::{RepoPath, TaskOutcome, is_plain};
use cortex_learn::JOINS_A_CLUSTER;
use cortex_store::{ChangeCardRecord, ChangedFile, EpisodeRecord, NodeId, Queries, StoreError};

use crate::disk::is_readable;
use crate::graph::Graph;
use crate::history_seeds::{SIMILAR_CHANGE_BAR, scored_changes};
use crate::prompt::PromptTerms;
use crate::similar::SimilarMatch;

pub use self::wording::{MAX_EXPERIENCE_TOKENS, MAX_HINT_FILES};
use self::wording::{age, fitted, listed};

/// The most past task records read for the hint, the newest first: a task like today's from
/// longer ago than this many tasks says less than one from this week.
pub const MAX_EPISODES_READ: usize = 200;

/// What the hint is looked for with.
#[derive(Debug, Clone, Copy)]
pub struct ExperienceAsk<'a> {
    /// When the task asking began, in UTC milliseconds: only work that ended strictly before it
    /// is read.
    pub before_ms: i64,
    /// What the similar-task seeds found for the prompt.
    pub similar: &'a SimilarMatch,
    /// The prompt's words, which past changes' cards are matched against.
    pub terms: &'a PromptTerms,
    /// The repository's root, where the files the hint names are looked for.
    pub root: &'a Path,
    /// When the search must stop, if it has not; a search it overtakes gives no hint.
    pub deadline: Option<Instant>,
}

/// The hint `ask` gives from the records and cards `queries` reads, their files as nodes of
/// `graph`: one line, ending its line, within [`MAX_EXPERIENCE_TOKENS`]; none when no earlier work
/// like the prompt can be told as it is now, or when the deadline overtook the search.
///
/// # Errors
///
/// The store's errors.
pub fn experience_hint(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    ask: &ExperienceAsk<'_>,
) -> Result<Option<String>, StoreError> {
    let found = match from_tasks(queries, graph, ask)? {
        Found::Hint(hint) => return Ok(Some(hint)),
        Found::OutOfTime => return Ok(None),
        Found::Nothing => from_changes(queries, ask)?,
    };
    Ok(match found {
        Found::Hint(hint) => Some(hint),
        Found::OutOfTime | Found::Nothing => None,
    })
}

/// What one source of earlier work came to.
enum Found {
    /// A hint.
    Hint(String),
    /// Nothing it could tell.
    Nothing,
    /// The deadline overtook it.
    OutOfTime,
}

/// Whether `deadline` has passed.
fn is_past(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| Instant::now() >= deadline)
}

/// The hint the newest past task like the prompt gives, of those read.
fn from_tasks(
    queries: Queries<'_>,
    graph: &Graph<NodeId>,
    ask: &ExperienceAsk<'_>,
) -> Result<Found, StoreError> {
    if ask.similar.files.is_empty() {
        return Ok(Found::Nothing);
    }
    // The store reads only what ended strictly before the task began: the temporal guard.
    for task in queries.episode_changes_ended_before(ask.before_ms, MAX_EPISODES_READ)? {
        // INVARIANT: deterministic (a search the clock cut short gives no hint, never a partial one)
        if is_past(ask.deadline) {
            return Ok(Found::OutOfTime);
        }
        let has_stood = matches!(task.outcome, TaskOutcome::Success | TaskOutcome::Unknown);
        if !has_stood || !is_like(&task.change, graph, ask.similar) {
            continue;
        }
        let Some(record) = queries.episode_of_task(task.task_id)? else {
            continue;
        };
        if let Some(hint) = task_hint(queries, &record, ask)? {
            return Ok(Found::Hint(hint));
        }
    }
    Ok(Found::Nothing)
}

/// The hint the best past change like the prompt gives.
fn from_changes(queries: Queries<'_>, ask: &ExperienceAsk<'_>) -> Result<Found, StoreError> {
    for (card, score) in scored_changes(queries, ask.terms)? {
        if is_past(ask.deadline) {
            return Ok(Found::OutOfTime);
        }
        // INVARIANT: silent-when-unsure (a change below the bar the seeds use is not like enough)
        if score < SIMILAR_CHANGE_BAR {
            continue;
        }
        // A commit's time is kept to the second: one in the task's first second may come after it.
        let committed_by_ms = card.committed_at_s.saturating_add(1).saturating_mul(1_000);
        if committed_by_ms > ask.before_ms {
            continue;
        }
        if let Some(hint) = change_hint(queries, &card, ask)? {
            return Ok(Found::Hint(hint));
        }
    }
    Ok(Found::Nothing)
}

/// Whether a task that changed `change` is like the prompt: at least [`JOINS_A_CLUSTER`] of the
/// files it changed are files of `graph` that `similar` gave.
fn is_like(change: &[ChangedFile], graph: &Graph<NodeId>, similar: &SimilarMatch) -> bool {
    let changed: BTreeSet<NodeId> = change.iter().map(|file| file.node).collect();
    let shared = changed
        .iter()
        .filter(|node| {
            graph
                .index_of(**node)
                .is_some_and(|index| similar.files.contains_key(&index))
        })
        .count();
    let (Ok(shared), Ok(count)) = (u32::try_from(shared), u32::try_from(changed.len())) else {
        return false;
    };
    // INVARIANT: silent-when-unsure (a task sharing less than this with like work is not told)
    count > 0 && f64::from(shared) >= JOINS_A_CLUSTER * f64::from(count)
}

/// What the task `record` says, as the files are now: the files it changed and kept, and an
/// approach of it that did not hold.
fn task_hint(
    queries: Queries<'_>,
    record: &EpisodeRecord,
    ask: &ExperienceAsk<'_>,
) -> Result<Option<String>, StoreError> {
    let undone = &record.pitfalls.undone;
    let kept: Vec<NodeId> = record
        .change
        .iter()
        .map(|file| file.node)
        .filter(|node| !undone.contains(node))
        .collect();
    let files = present(queries, &kept, ask.root)?;
    let lead = format!(
        "A similar task ({})",
        age(ask.before_ms.saturating_sub(record.created_at_ms))
    );
    let approach = approach(queries, record, ask.root)?;
    Ok(fitted(&lead, &files, approach.as_deref()))
}

/// What the change `card` says, as the files are now: the files it changed.
fn change_hint(
    queries: Queries<'_>,
    card: &ChangeCardRecord,
    ask: &ExperienceAsk<'_>,
) -> Result<Option<String>, StoreError> {
    let Some(short) = card.oid.get(..7).filter(|short| is_plain(short)) else {
        return Ok(None);
    };
    let files = present(queries, &card.files, ask.root)?;
    let committed_ms = card.committed_at_s.saturating_mul(1_000);
    let lead = format!(
        "A similar change (commit {short}, {})",
        age(ask.before_ms.saturating_sub(committed_ms))
    );
    Ok(fitted(&lead, &files, None))
}

/// One approach of the task `record` that did not hold, as a clause: its first failure that
/// stayed the same until another file changed, else the files it put back. A failure or a set of
/// files any of whose files is gone, or that names more than [`MAX_HINT_FILES`], is not said.
fn approach(
    queries: Queries<'_>,
    record: &EpisodeRecord,
    root: &Path,
) -> Result<Option<String>, StoreError> {
    for failure in &record.pitfalls.persisted {
        if !is_plain(failure.runner.as_str()) {
            continue;
        }
        let Some(edited) = all_present(queries, &failure.edited, root)? else {
            continue;
        };
        let Some(until) = all_present(queries, &failure.until, root)? else {
            continue;
        };
        return Ok(Some(format!(
            "editing {} alone left the same {} tests failing until {} changed too",
            listed(&edited),
            failure.runner.as_str(),
            listed(&until)
        )));
    }
    let undone = &record.pitfalls.undone;
    if undone.is_empty() {
        return Ok(None);
    }
    Ok(all_present(queries, undone, root)?.map(|back| format!("it put {} back", listed(&back))))
}

/// The files of `nodes` the work can name now, in their order: a file node the store still
/// holds, whose path is plain and is a file in the repository at `root`. A file gone from either
/// is left out.
fn present(
    queries: Queries<'_>,
    nodes: &[NodeId],
    root: &Path,
) -> Result<Vec<RepoPath>, StoreError> {
    let names = queries.anchor_names(nodes)?;
    let mut paths: Vec<RepoPath> = Vec::new();
    for node in nodes {
        // INVARIANT: true-at-render (a file the hint names is there when the hint is written)
        let Some(path) = names
            .get(node)
            .and_then(|name| RepoPath::new(name).ok())
            .filter(|path| is_plain(path.as_str()) && is_readable(root, path))
        else {
            continue;
        };
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

/// Every file of `nodes`, as [`present`] finds them, when all of them are there and they are at
/// most [`MAX_HINT_FILES`]: a fact missing one of its files would say something else.
fn all_present(
    queries: Queries<'_>,
    nodes: &[NodeId],
    root: &Path,
) -> Result<Option<Vec<RepoPath>>, StoreError> {
    if nodes.is_empty() || nodes.len() > MAX_HINT_FILES {
        return Ok(None);
    }
    let paths = present(queries, nodes, root)?;
    Ok((paths.len() == nodes.len()).then_some(paths))
}
