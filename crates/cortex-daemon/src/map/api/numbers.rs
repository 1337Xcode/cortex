//! The numbers the store holds about each listed file: interest, tokens, last touch, facts,
//! reads and commits, each read once through the store's own queries.
//!
//! Where a number is not stored as the API gives it, the module says how it is made from what is:
//! interest is the stored score brought forward to now and set against the highest file's, and
//! tokens are those of the responses given while an agent worked on the file.

use cortex_core::FactStatus;
use cortex_learn::{FADED_OUT_BELOW, decayed};
use cortex_retrieve::GraphSettings;
use cortex_store::{FileId, Queries, StoreError};

use super::listing::Listing;

/// What the store holds about one file; every count is 0 and every time absent for a file no
/// agent has worked on.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct FileNumbers {
    /// How much the file matters now, from 0 to 1: its stored interest brought forward to the
    /// latest task, over the highest such interest among the files; 0 when no file has any.
    pub(super) interest: f64,
    /// The tokens the responses took while an agent worked on the file, over all time.
    pub(super) tokens: u64,
    /// When an agent last read or edited it, in UTC milliseconds.
    pub(super) touched_ms: Option<i64>,
    /// How many facts (active, candidate and suspect) are about it.
    pub(super) facts: u64,
    /// How many of those are suspect.
    pub(super) suspect: u64,
    /// How many times agents read it.
    pub(super) reads: u64,
    /// How many commits of the mined history changed it.
    pub(super) commits: u64,
}

/// The numbers of every file of `listing`, by id, read through `queries`: `latest_seq` is the
/// latest task's number, which scores are brought forward to, weighed by `settings`.
///
/// # Errors
///
/// SQLite errors and stored values that cannot be read back.
pub(super) fn read(
    queries: Queries<'_>,
    listing: &Listing,
    (settings, latest_seq): (&GraphSettings, u64),
) -> Result<Vec<FileNumbers>, StoreError> {
    let mut numbers = vec![FileNumbers::default(); listing.files().len()];
    add_interest(queries, listing, (settings, latest_seq), &mut numbers)?;
    for row in queries.file_reads()? {
        if let Some(entry) = at(listing, &mut numbers, row.node) {
            entry.reads = row.reads;
            entry.touched_ms = Some(row.last_ms);
        }
    }
    for (node, tokens) in queries.file_tokens(0)? {
        if let Some(entry) = at(listing, &mut numbers, node) {
            entry.tokens = tokens;
        }
    }
    for row in queries.fact_counts_by_file()? {
        if let Some(entry) = at(listing, &mut numbers, row.file) {
            entry.facts += row.count;
            if row.status == FactStatus::Suspect {
                entry.suspect += row.count;
            }
        }
    }
    let files: Vec<FileId> = listing.files().iter().map(|file| file.file).collect();
    for row in queries.file_commits_of(&files)? {
        if let Some(entry) = listing
            .id_of_row(row.file)
            .and_then(|id| numbers.get_mut(id))
        {
            entry.commits = u64::from(row.count);
        }
    }
    Ok(numbers)
}

/// The entry of `numbers` for the file whose node is `node`, if the map lists one.
fn at<'a>(
    listing: &Listing,
    numbers: &'a mut [FileNumbers],
    node: cortex_store::NodeId,
) -> Option<&'a mut FileNumbers> {
    listing.id_of_node(node).and_then(|id| numbers.get_mut(id))
}

/// Sets each file's interest: its stored score brought forward to `latest_seq` by the interest
/// decay of `settings`, a score that faded below the floor counting as none, over the highest.
fn add_interest(
    queries: Queries<'_>,
    listing: &Listing,
    (settings, latest_seq): (&GraphSettings, u64),
    numbers: &mut [FileNumbers],
) -> Result<(), StoreError> {
    let mut scores = Vec::new();
    for row in queries.file_interest_from(FADED_OUT_BELOW)? {
        let now = decayed(row.score, row.last_seq, latest_seq, settings.interest_decay);
        if now.is_finite() && now >= FADED_OUT_BELOW {
            scores.push((row.node, now));
        }
    }
    let highest = scores.iter().map(|(_, score)| *score).fold(0.0, f64::max);
    if highest <= 0.0 {
        return Ok(());
    }
    for (node, score) in scores {
        if let Some(entry) = at(listing, numbers, node) {
            entry.interest = (score / highest).clamp(0.0, 1.0);
        }
    }
    Ok(())
}
