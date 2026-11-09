//! The line a read gets naming the file earlier tasks read next: when an agent reads file A, and
//! earlier tasks here that read A before changing anything went on to one file far more than to
//! any other, that file is named, once a task. Silent unless the way is strong: its weight, as the
//! tasks' outcomes added to it and faded since, is at least [`NEXT_FILE_MIN_WEIGHT`] and at least
//! [`NEXT_FILE_MIN_SHARE`] of all the weight leaving A. The file named is one the index holds as it
//! is on disk, the task has not read or changed it yet, and no earlier line of the task named it.

use std::path::Path;

use cortex_core::{EventKind, ReasonKind, RepoPath, Ulid, is_plain};
use cortex_learn::decayed;
use cortex_store::{NodeId, Queries, StoreError, TransitionRow};

use crate::disk::CheckedFile;

/// The least faded weight a way from one file to the next must hold to be named: two tasks seen to
/// work, or about seven nobody could vouch for, with little fading since.
pub const NEXT_FILE_MIN_WEIGHT: f64 = 2.0;

/// The least share of all the weight leaving a file that the way named must hold: tasks that went
/// on to it at least as often as to every other file together.
pub const NEXT_FILE_MIN_SHARE: f64 = 0.5;

/// How many of a task's latest events are looked at for whether it read or changed the file.
const EVENTS_LOOKED_AT: usize = 2_000;

/// What the line is worked out for.
#[derive(Debug, Clone, Copy)]
pub struct NextQuery<'a> {
    /// The repository's root, where files are read from.
    pub root: &'a Path,
    /// The file being read.
    pub path: &'a RepoPath,
    /// The task reading it.
    pub task: Ulid,
    /// The repository's latest task number, which the weights fade to.
    pub seq: u64,
    /// `learning.link_decay`: what a way keeps of its weight per task.
    pub link_decay: f64,
    /// `index.max_file_bytes`: the most bytes the file named may hold to be read.
    pub max_file_bytes: u64,
}

/// The line, and the file it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextFile {
    /// The file named.
    pub node: NodeId,
    /// The line, ending in a line break.
    pub line: String,
}

/// The line the read `asked` describes gets, as the store `queries` reads holds what came before
/// it; none when the way is not strong enough or the file it leads to cannot be named, as the
/// module says.
///
/// # Errors
///
/// The store's errors.
pub fn next_file(
    queries: Queries<'_>,
    asked: &NextQuery<'_>,
) -> Result<Option<NextFile>, StoreError> {
    let Some(from) = queries.file_node_at(asked.path)? else {
        return Ok(None);
    };
    let ways = queries.transitions_from(&[from])?;
    let Some((best, weight, total)) = strongest(&ways, asked) else {
        return Ok(None);
    };
    // INVARIANT: silent-when-unsure (a way too weak, or not plainly the main one, names nothing)
    if weight < NEXT_FILE_MIN_WEIGHT || weight < NEXT_FILE_MIN_SHARE * total {
        return Ok(None);
    }
    let names = queries.anchor_names(&[best.dst])?;
    let Some(to) = names
        .get(&best.dst)
        .and_then(|name| RepoPath::new(name).ok())
    else {
        return Ok(None);
    };
    if !is_plain(to.as_str()) || !is_plain(asked.path.as_str()) || !is_on_disk(queries, &to, asked)?
    {
        return Ok(None);
    }
    if is_touched(queries, asked.task, best.dst)? || is_named(queries, asked.task, best.dst)? {
        return Ok(None);
    }
    let times: u32 = ways.iter().map(|way| way.count).sum();
    let line = format!(
        "[cortex] After reading {}, earlier tasks here went on to {} ({} of {times} times).\n",
        asked.path.as_str(),
        to.as_str(),
        best.count
    );
    Ok(Some(NextFile {
        node: best.dst,
        line,
    }))
}

/// The way of `ways` holding the most weight faded to `asked`'s task number, the smaller node
/// first on a tie, with that weight and the weight of all of them.
fn strongest<'a>(
    ways: &'a [TransitionRow],
    asked: &NextQuery<'_>,
) -> Option<(&'a TransitionRow, f64, f64)> {
    let faded =
        |way: &TransitionRow| decayed(way.weight, way.last_seq, asked.seq, asked.link_decay);
    let total: f64 = ways.iter().map(faded).sum();
    ways.iter()
        .map(|way| (way, faded(way)))
        .max_by(|(left, left_weight), (right, right_weight)| {
            left_weight
                .total_cmp(right_weight)
                .then_with(|| right.dst.cmp(&left.dst))
        })
        .map(|(way, weight)| (way, weight, total))
}

/// Whether the file at `path` is on disk as the index read it.
fn is_on_disk(
    queries: Queries<'_>,
    path: &RepoPath,
    asked: &NextQuery<'_>,
) -> Result<bool, StoreError> {
    let Some(row) = queries.file(path)? else {
        return Ok(false);
    };
    let Some(hash) = row.file.content_hash else {
        return Ok(false);
    };
    // INVARIANT: true-at-render (the file named is there, as the index read it)
    Ok(CheckedFile::read(asked.root, path, &hash, asked.max_file_bytes).is_ok())
}

/// Whether `task` read or changed the file `node` already.
fn is_touched(queries: Queries<'_>, task: Ulid, node: NodeId) -> Result<bool, StoreError> {
    Ok(queries
        .latest_events_of_task(task, EVENTS_LOOKED_AT)?
        .iter()
        .any(|row| {
            row.record.node_id == Some(node)
                && matches!(row.record.kind, EventKind::Read | EventKind::Edit)
        }))
}

/// Whether an earlier line of `task` named the file `node`.
fn is_named(queries: Queries<'_>, task: Ulid, node: NodeId) -> Result<bool, StoreError> {
    Ok(queries.injections_of_task(task)?.iter().any(|row| {
        row.record
            .files
            .iter()
            .any(|file| file.node == node && file.reason == Some(ReasonKind::ReadNext))
    }))
}
