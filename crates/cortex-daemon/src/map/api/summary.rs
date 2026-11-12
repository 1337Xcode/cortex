//! `api/summary`: the numbers of the map's first screen: how many files and folders are indexed,
//! how many agents have read, whether anything has been learned at all, the pair of files most
//! strongly changed together (ranked as the graph ranks its links, so the map always draws it), and
//! the folder agents spent the most tokens in lately; and the limits of the history Cortex reads,
//! which every number of commits on the map counts within.
//!
//! Files are those `api/graph` lists, and a file's id names it in both. The only time-dependent
//! number, the busiest folder, reads the clock through [`Source::now_ms`] alone.

use std::collections::{BTreeMap, BTreeSet};

use axum::http::StatusCode;
use axum::response::Response;
use cortex_store::{Queries, StoreError};
use serde::Serialize;

use super::Ask;
use super::links;
use super::listing::Listing;
use super::reply::{Failure, json, respond};
use crate::map::settings::HistoryLimits;
use crate::map::site::Site;
use crate::map::source::Source;

/// The version of the body's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// How many days back the busiest folder looks. A week is long enough for a folder to stand out
/// from one afternoon's work and short enough that last month's does not hide this week's.
const WINDOW_DAYS: u64 = 7;

/// The milliseconds in a day.
const DAY_MS: i64 = 86_400_000;

/// The body.
#[derive(Debug, Serialize)]
struct Summary {
    /// The shape's version.
    version: u8,
    /// How many files are listed.
    files: usize,
    /// How many folders hold them, at any depth, the root not counted.
    folders: usize,
    /// How many files agents have read.
    explored_files: usize,
    /// Whether any link, fact, read or token is stored.
    learned: bool,
    /// The two files that changed together most strongly, as the graph ranks its links.
    strongest_pair: Option<Pair>,
    /// The folder whose own files took the most tokens in the window.
    busiest_folder: Option<Busiest>,
    /// How many days back the busiest folder looks.
    window_days: u64,
    /// The history the numbers of commits count.
    history: HistoryLimits,
}

/// Two files that changed together.
#[derive(Debug, Serialize)]
struct Pair {
    /// The file with the smaller id.
    from: usize,
    /// The file with the larger id.
    to: usize,
    /// The commits that changed both.
    commits: u64,
}

/// A folder and the tokens its files took.
#[derive(Debug, Serialize)]
struct Busiest {
    /// The folder's path; empty for the repository's top folder.
    path: String,
    /// The tokens its own files took in the window.
    tokens: u64,
}

/// The summary of the store `site` reads, as JSON.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    respond(site, "reading the map's summary", || {
        if ask.tail.is_some() {
            return Err(Failure::Refused(StatusCode::NOT_FOUND));
        }
        let source = site.source();
        let listing = site.listing()?;
        let reader = source.open()?;
        let summary = reader.consistently(|queries| summary_of(queries, source, &listing))?;
        json(&summary)
    })
}

/// The summary `queries` read.
fn summary_of(
    queries: Queries<'_>,
    source: &Source,
    listing: &Listing,
) -> Result<Summary, StoreError> {
    let reads = queries.file_reads()?;
    let explored_files = reads
        .iter()
        .filter(|row| row.reads > 0)
        .filter(|row| listing.id_of_node(row.node).is_some())
        .count();
    let strongest_pair = links::strongest_co_change(queries, listing)?.map(|link| Pair {
        from: link.from,
        to: link.to,
        commits: link.count,
    });
    let learned = !reads.is_empty()
        || strongest_pair.is_some()
        || !queries.file_tokens(0)?.is_empty()
        || !queries.fact_counts_by_file()?.is_empty()
        || !queries.coaccess_links_after(None, 1)?.is_empty()
        || !queries.delivered_work_orders()?.is_empty();
    Ok(Summary {
        version: VERSION,
        files: listing.files().len(),
        folders: folder_count(listing),
        explored_files,
        learned,
        strongest_pair,
        busiest_folder: busiest_folder(queries, listing, source)?,
        window_days: WINDOW_DAYS,
        history: source.history_limits(),
    })
}

/// How many folders hold the listed files, any ancestor of a file counting once.
fn folder_count(listing: &Listing) -> usize {
    let mut folders: BTreeSet<&str> = BTreeSet::new();
    for file in listing.files() {
        let path = file.path.as_str();
        for (at, _) in path.match_indices('/') {
            if let Some(folder) = path.get(..at) {
                folders.insert(folder);
            }
        }
    }
    folders.len()
}

/// The folder whose own files took the most tokens since the window began, the first by path on a
/// tie; none when no file took any.
fn busiest_folder(
    queries: Queries<'_>,
    listing: &Listing,
    source: &Source,
) -> Result<Option<Busiest>, StoreError> {
    let window_ms = i64::try_from(WINDOW_DAYS)
        .unwrap_or(i64::MAX)
        .saturating_mul(DAY_MS);
    // INVARIANT: deterministic (the clock enters the summary here and nowhere else)
    let since = source.now_ms().saturating_sub(window_ms);
    let tokens = queries.file_tokens(since)?;
    let by_file = tokens.iter().filter_map(|&(node, tokens)| {
        let file = listing.id_of_node(node).and_then(|id| listing.get(id))?;
        Some((file.path.as_str(), tokens))
    });
    Ok(busiest_of(by_file).map(|(path, tokens)| Busiest {
        path: path.to_owned(),
        tokens,
    }))
}

/// The folder `path` is in: what comes before its last `/`, or the empty path of the repository's
/// top folder for a file there.
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// Of the tokens each file took, by its path, the folder whose own files took the most, with
/// their sum, the first by path on a tie; none when no file took any.
fn busiest_of<'a>(by_file: impl IntoIterator<Item = (&'a str, u64)>) -> Option<(&'a str, u64)> {
    let mut folders: BTreeMap<&str, u64> = BTreeMap::new();
    for (path, tokens) in by_file {
        *folders.entry(folder_of(path)).or_insert(0) += tokens;
    }
    let mut busiest: Option<(&str, u64)> = None;
    for (folder, tokens) in folders {
        if tokens > 0 && busiest.is_none_or(|(_, most)| tokens > most) {
            busiest = Some((folder, tokens));
        }
    }
    busiest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file at the root counts for the top folder, the empty path: `README.md`'s 500 tokens beat
    /// `src`'s 300 + 100 = 400 and `src/api`'s 450, which a subfolder's files do not add to `src`.
    #[test]
    fn the_top_folder_can_be_the_busiest() {
        let tokens = [
            ("README.md", 500),
            ("src/main.rs", 300),
            ("src/lib.rs", 100),
            ("src/api/routes.rs", 450),
        ];
        assert_eq!(busiest_of(tokens), Some(("", 500)));
    }

    /// Without the root file, `src/api`'s 450 beats `src`'s own 400; on a tie the first path wins;
    /// with no tokens there is none.
    #[test]
    fn the_busiest_folder_counts_its_own_files() {
        let tokens = [
            ("src/main.rs", 300),
            ("src/lib.rs", 100),
            ("src/api/routes.rs", 450),
        ];
        assert_eq!(busiest_of(tokens), Some(("src/api", 450)));
        assert_eq!(busiest_of([("b/x.rs", 7), ("a/y.rs", 7)]), Some(("a", 7)));
        assert_eq!(busiest_of([("a/y.rs", 0)]), None);
    }
}
