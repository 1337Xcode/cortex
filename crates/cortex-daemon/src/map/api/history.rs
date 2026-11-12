//! Why a file is the way it is, as the daemon already holds it: the commits the stored blames of
//! the file name, each with the subject the history worker read and the pull request and issues
//! its card names; how many such commits there are, as the list names only the newest; and
//! whether the history worker reads blames at all, as with it off no file ever has any.
//!
//! Nothing is worked out here. A blame is stored for the file's content as it stands, so a file
//! changed since has none until the worker blames it again; a commit whose subject the worker's
//! cache no longer holds is left out rather than read from git; line numbers are not given, as
//! they may have moved.

use std::collections::BTreeSet;

use cortex_core::ipc::HistoryReport;
use cortex_store::{ChangeCardRecord, Queries, StoreError};
use serde::Serialize;

use super::listing::ListedFile;
use crate::map::source::Source;

/// The most commits the answer names: the ten that changed the file last.
pub(super) const MAX_HISTORY: usize = 10;

/// The line range that covers every line a stored blame can have.
const ALL_LINES: (u32, u32) = (1, u32::MAX);

/// Whether the history worker reads blames, so that a file's history can fill in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Reader {
    /// It runs: a file has a history once a prompt or upkeep asked for its blame.
    On,
    /// It does not run: `history.why` and `history.seeds` are both off.
    Off,
    /// It runs but git is refused in the repository, so it blames nothing.
    Refused,
}

/// What the history worker is, as `source` reports it.
pub(super) fn reader_of(source: &Source) -> Reader {
    reader_from(source.history_report().as_ref())
}

/// What the history worker is, from its report: none while it does not run.
pub(super) fn reader_from(report: Option<&HistoryReport>) -> Reader {
    match report {
        None => Reader::Off,
        Some(report) if report.refused.is_some() => Reader::Refused,
        Some(_) => Reader::On,
    }
}

/// The commits behind a file's lines that the panel can name.
#[derive(Debug, Default)]
pub(super) struct FileHistory {
    /// The newest of them, newest first, at most [`MAX_HISTORY`].
    pub(super) commits: Vec<Commit>,
    /// How many there are in all.
    pub(super) total: usize,
}

/// One commit that last changed some lines of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Commit {
    /// The commit's subject, as the worker read it.
    subject: String,
    /// The pull request and issues its card names, as written and separated by spaces; empty when
    /// it names none.
    refs: String,
    /// When it was committed, in UTC seconds.
    date_s: i64,
}

/// The commits the stored blames of `file` name whose subjects `source` holds: how many, and the
/// newest [`MAX_HISTORY`] of them, newest first.
///
/// # Errors
///
/// SQLite errors and stored values that cannot be read back.
pub(super) fn of_file(
    queries: Queries<'_>,
    source: &Source,
    file: &ListedFile,
) -> Result<FileHistory, StoreError> {
    let Some(row) = queries.file(&file.path)? else {
        return Ok(FileHistory::default());
    };
    let Some(hash) = row.file.content_hash else {
        return Ok(FileHistory::default());
    };
    let blames = queries.cached_blames((&file.path, &hash), ALL_LINES.0, ALL_LINES.1)?;
    let oids: BTreeSet<String> = blames
        .into_iter()
        .flat_map(|blame| blame.runs)
        .map(|run| run.oid)
        .collect();
    let oids: Vec<String> = oids.into_iter().collect();
    let mut cards = queries.change_cards(&oids)?;
    cards.sort_by(|a, b| {
        b.committed_at_s
            .cmp(&a.committed_at_s)
            .then_with(|| a.oid.cmp(&b.oid))
    });
    let mut commits: Vec<Commit> = cards
        .iter()
        .filter_map(|card| commit_of(source, card))
        .collect();
    let total = commits.len();
    commits.truncate(MAX_HISTORY);
    Ok(FileHistory { commits, total })
}

/// `card` as a commit, when the subject is held.
fn commit_of(source: &Source, card: &ChangeCardRecord) -> Option<Commit> {
    let subject = source.commit_subject(&card.oid)?;
    let mut refs: Vec<String> = Vec::new();
    if let Some(number) = card.pull_request {
        refs.push(format!("#{number}"));
    }
    for issue in &card.issues {
        if !refs.contains(issue) {
            refs.push(issue.clone());
        }
    }
    Some(Commit {
        subject,
        refs: refs.join(" "),
        date_s: card.committed_at_s,
    })
}
