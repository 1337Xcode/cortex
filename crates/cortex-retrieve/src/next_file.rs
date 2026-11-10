//! The line a read gets naming the file earlier tasks read next: when an agent reads file A, and
//! earlier tasks here that read A before changing anything went on to one file far more than to
//! any other, that file is named, once a task. Silent unless the way is strong: its weight, as the
//! tasks' outcomes added to it and faded since, is at least [`NEXT_FILE_MIN_WEIGHT`] and more than
//! [`NEXT_FILE_MAJORITY`] of all the weight leaving A, so an even split names nothing. The file
//! named is one the index holds as code (not generated, vendored, binary or named like a secret)
//! and as it is on disk; the task has not read or changed it yet, and no earlier line that reached
//! the task named it.
//!
//! The checks run cheapest first: the ways from A, two indexed lookups of the task's events and
//! texts, the file's flags. What passes is a [`NextCandidate`], whose line the caller can weigh
//! against its cap and claim before [`NextCandidate::checked`] reads and hashes the file, last.

use std::path::Path;

use cortex_core::{ContentHash, FileFlags, ReasonKind, RepoPath, Ulid, is_plain};
use cortex_learn::decayed;
use cortex_store::{FileRow, NodeId, Queries, StoreError, TransitionRow};

use crate::disk::CheckedFile;

/// The least faded weight a way from one file to the next must hold to be named: two tasks seen to
/// work, or about seven nobody could vouch for, with little fading since.
pub const NEXT_FILE_MIN_WEIGHT: f64 = 2.0;

/// The share of all the weight leaving a file that the way named must hold more than: tasks went
/// on to it more often than to every other file together.
pub const NEXT_FILE_MAJORITY: f64 = 0.5;

/// How many of a task's latest events are looked at for whether it read or changed the file.
const EVENTS_LOOKED_AT: usize = 2_000;

/// What a file is flagged as that keeps it from being named: no code an agent works on, or a file
/// Cortex never opens.
const NOT_NAMED: [FileFlags; 4] = [
    FileFlags::GENERATED,
    FileFlags::VENDORED,
    FileFlags::BINARY,
    FileFlags::SECRET_PATTERN,
];

/// What the line is worked out for.
#[derive(Debug, Clone, Copy)]
pub struct NextQuery<'a> {
    /// The file being read.
    pub path: &'a RepoPath,
    /// The task reading it.
    pub task: Ulid,
    /// The repository's latest task number, which the weights fade to.
    pub seq: u64,
    /// `learning.link_decay`: what a way keeps of its weight per task.
    pub link_decay: f64,
}

/// A line that passed every check but the file's own on disk: its file named, the line, and the
/// file as the index holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextCandidate {
    /// The file named.
    node: NodeId,
    /// The line, ending in a line break.
    line: String,
    /// The file's path.
    path: RepoPath,
    /// The hash of the file as the index read it.
    hash: ContentHash,
}

/// The line, its file checked on disk, and the file it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextFile {
    /// The file named.
    pub node: NodeId,
    /// The line, ending in a line break.
    pub line: String,
}

impl NextCandidate {
    /// The file the line names.
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The line, ending in a line break, not to be given before [`NextCandidate::checked`].
    #[must_use]
    pub fn line(&self) -> &str {
        &self.line
    }

    /// The line, once its file in the repository at `root` is on disk as the index read it and
    /// holds at most `max_file_bytes`; none otherwise.
    #[must_use]
    pub fn checked(self, root: &Path, max_file_bytes: u64) -> Option<NextFile> {
        // INVARIANT: true-at-render (the file named is there, as the index read it)
        CheckedFile::read(root, &self.path, &self.hash, max_file_bytes).ok()?;
        Some(NextFile {
            node: self.node,
            line: self.line,
        })
    }
}

/// The line the read `asked` describes gets, as the store `queries` reads holds what came before
/// it, its file still to be checked on disk; none when the way is not strong enough or the file it
/// leads to cannot be named, as the module says.
///
/// # Errors
///
/// The store's errors.
pub fn next_file(
    queries: Queries<'_>,
    asked: &NextQuery<'_>,
) -> Result<Option<NextCandidate>, StoreError> {
    if !is_plain(asked.path.as_str()) {
        return Ok(None);
    }
    let Some(from) = queries.file_node_at(asked.path)? else {
        return Ok(None);
    };
    let ways = queries.transitions_from(&[from])?;
    let Some((best, weight, total)) = strongest(&ways, asked) else {
        return Ok(None);
    };
    // INVARIANT: silent-when-unsure (a way too weak, or not plainly the main one, names nothing)
    if weight < NEXT_FILE_MIN_WEIGHT || weight <= NEXT_FILE_MAJORITY * total {
        return Ok(None);
    }
    if queries.has_named(asked.task, best.dst, ReasonKind::ReadNext)?
        || queries.has_touched(asked.task, best.dst, EVENTS_LOOKED_AT)?
    {
        return Ok(None);
    }
    let names = queries.anchor_names(&[best.dst])?;
    let Some(to) = names
        .get(&best.dst)
        .and_then(|name| RepoPath::new(name).ok())
        .filter(|to| is_plain(to.as_str()))
    else {
        return Ok(None);
    };
    let Some(hash) = queries.file(&to)?.as_ref().and_then(nameable_hash) else {
        return Ok(None);
    };
    let line = format!(
        "[cortex] After reading {}, earlier tasks here went on to read {} ({} times).\n",
        asked.path.as_str(),
        to.as_str(),
        best.count
    );
    Ok(Some(NextCandidate {
        node: best.dst,
        line,
        path: to,
        hash,
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

/// The hash the index stored for `file`, when the file may be named: one flagged [`NOT_NAMED`] is
/// never opened.
fn nameable_hash(file: &FileRow) -> Option<ContentHash> {
    let flags = file.file.flags;
    if NOT_NAMED.iter().any(|flag| flags.contains(*flag)) {
        return None;
    }
    file.file.content_hash
}
