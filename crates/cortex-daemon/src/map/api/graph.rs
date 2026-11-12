//! `api/graph`: the whole map in one body: every file with the numbers the store holds about it,
//! and the strongest links between files.
//!
//! Files come sorted by path in byte order with their index as `id`; links name files by that id.
//! Everything comes from one read transaction, so the numbers agree with each other and the
//! writer is held up for nothing.

use axum::response::Response;
use cortex_store::{Queries, StoreError};
use serde::Serialize;

use super::Ask;
use super::links::{self, Link};
use super::listing::Listing;
use super::numbers::{self, FileNumbers};
use super::reply::{Failure, json, respond};
use crate::map::site::Site;
use crate::map::source::Source;

/// The version of the body's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// The body.
#[derive(Debug, Serialize)]
struct Graph {
    /// The shape's version.
    version: u8,
    /// Every file, by path.
    files: Vec<FileEntry>,
    /// The strongest links, by kind, then file.
    links: Vec<LinkEntry>,
}

/// One file in the body.
#[derive(Debug, Serialize)]
struct FileEntry {
    /// Its index in the list.
    id: usize,
    /// Its path, relative to the repository's root.
    path: String,
    /// Its lines.
    lines: u64,
    /// How much it matters now, from 0 to 1.
    interest: f64,
    /// The tokens the responses took while an agent worked on it.
    tokens: u64,
    /// When an agent last read or edited it, in UTC milliseconds.
    touched_ms: Option<i64>,
    /// How many facts are about it.
    facts: u64,
    /// How many of those are suspect.
    suspect: u64,
    /// How many times agents read it.
    reads: u64,
    /// How many commits changed it.
    commits: u64,
}

/// One link in the body.
#[derive(Debug, Serialize)]
struct LinkEntry {
    /// The file with the smaller id.
    from: usize,
    /// The file with the larger id.
    to: usize,
    /// What the link is made of.
    kind: &'static str,
    /// How strong it is, from 0 to 1.
    weight: f64,
    /// For a co-change link, the commits that changed both files.
    commits: u64,
}

/// The graph of the store `site` reads, as JSON.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    respond(site, "reading the map's graph", || {
        if ask.tail.is_some() {
            return Err(Failure::Refused(axum::http::StatusCode::NOT_FOUND));
        }
        let source = site.source();
        let listing = site.listing()?;
        let graph = read(source, &listing)?;
        json(&graph)
    })
}

/// The graph the store holds, read in one transaction.
fn read(source: &Source, listing: &Listing) -> Result<Graph, StoreError> {
    let reader = source.open()?;
    reader.consistently(|queries| graph_of(queries, source, listing))
}

/// The graph `queries` read.
fn graph_of(queries: Queries<'_>, source: &Source, listing: &Listing) -> Result<Graph, StoreError> {
    let latest_seq = queries.latest_task_seq()?;
    let weighing = (source.settings(), latest_seq);
    let numbers = numbers::read(queries, listing, weighing)?;
    let links = links::read(queries, listing, weighing)?;
    let files = listing
        .files()
        .iter()
        .zip(numbers)
        .enumerate()
        .map(|(id, (file, numbers))| entry(id, file.path.as_str(), file.lines, numbers))
        .collect();
    Ok(Graph {
        version: VERSION,
        files,
        links: links.iter().map(link_entry).collect(),
    })
}

/// The entry of the file with id `id`, at `path` with `lines` lines and `numbers`.
fn entry(id: usize, path: &str, lines: u64, numbers: FileNumbers) -> FileEntry {
    FileEntry {
        id,
        path: path.to_owned(),
        lines,
        interest: numbers.interest,
        tokens: numbers.tokens,
        touched_ms: numbers.touched_ms,
        facts: numbers.facts,
        suspect: numbers.suspect,
        reads: numbers.reads,
        commits: numbers.commits,
    }
}

/// The entry of `link`.
fn link_entry(link: &Link) -> LinkEntry {
    LinkEntry {
        from: link.from,
        to: link.to,
        kind: link.kind.name(),
        weight: link.weight,
        commits: link.commits,
    }
}
