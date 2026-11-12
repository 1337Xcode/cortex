//! `api/file/<id>`: what the map's side panel shows of one file: the facts about it, the commits
//! behind its code, and the times Cortex named it to an agent.
//!
//! `<id>` is the file's id in `api/graph`, which holds only while the files do; an id that names
//! no file is 404 with an empty body. Links of the file are in `api/graph` and are not repeated.

use axum::http::StatusCode;
use axum::response::Response;
use cortex_core::FactStatus;
use cortex_store::{Queries, StoreError};
use serde::Serialize;

use super::Ask;
use super::history::{self, Commit};
use super::listing::ListedFile;
use super::reply::{Failure, json, respond};
use super::told::{self, Told};
use crate::map::site::Site;
use crate::map::source::Source;

/// The version of the body's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// The statuses a fact is shown in, in the order its statuses are listed; a retired fact is gone.
const SHOWN: [FactStatus; 3] = [
    FactStatus::Active,
    FactStatus::Candidate,
    FactStatus::Suspect,
];

/// The most digits an id is written with: more could not be the index of any file the store holds.
const MAX_ID_DIGITS: usize = 10;

/// The body.
#[derive(Debug, Serialize)]
struct Panel {
    /// The shape's version.
    version: u8,
    /// The file's id.
    id: usize,
    /// The file's path.
    path: String,
    /// The facts about it, by id.
    facts: Vec<Fact>,
    /// The commits behind its code, newest first.
    history: Vec<Commit>,
    /// The times Cortex named it, latest first.
    told: Vec<Told>,
}

/// One fact about the file.
#[derive(Debug, Serialize)]
struct Fact {
    /// Its number.
    id: i64,
    /// What kind of rule it is.
    kind: &'static str,
    /// Where it stands: active, candidate or suspect.
    status: &'static str,
    /// What it says, as stored (at most 240 characters).
    text: String,
}

/// The panel of the file the request names, as JSON.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    respond(site, "reading a file of the map", || {
        let Some(id) = ask.tail.as_deref().and_then(id_of) else {
            return Err(Failure::Refused(StatusCode::NOT_FOUND));
        };
        let listing = site.listing()?;
        let Some(file) = listing.get(id) else {
            return Err(Failure::Refused(StatusCode::NOT_FOUND));
        };
        let source = site.source();
        let reader = source.open()?;
        let panel = reader.consistently(|queries| panel_of(queries, source, (id, file)))?;
        json(&panel)
    })
}

/// The id `text` writes: digits only, none of them a sign, and few enough to be an index.
fn id_of(text: &str) -> Option<usize> {
    if text.is_empty()
        || text.len() > MAX_ID_DIGITS
        || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    text.parse().ok()
}

/// The panel of the file `file`, which has id `id`.
fn panel_of(
    queries: Queries<'_>,
    source: &Source,
    (id, file): (usize, &ListedFile),
) -> Result<Panel, StoreError> {
    Ok(Panel {
        version: VERSION,
        id,
        path: file.path.as_str().to_owned(),
        facts: facts_of(queries, file)?,
        history: history::of_file(queries, source, file)?,
        told: told::of_file(queries, file.node)?,
    })
}

/// The facts about `file`: active, candidate and suspect, by number.
fn facts_of(queries: Queries<'_>, file: &ListedFile) -> Result<Vec<Fact>, StoreError> {
    let mut facts = Vec::new();
    for status in SHOWN {
        for fact in queries.facts_in_files(&[file.node], status)? {
            if let Some(row) = queries.fact(fact)? {
                facts.push(Fact {
                    id: row.id.get(),
                    kind: row.record.kind.name(),
                    status: status.name(),
                    text: row.record.text,
                });
            }
        }
    }
    facts.sort_by_key(|fact| fact.id);
    Ok(facts)
}
