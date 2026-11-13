//! `api/search?q=<text>`: files whose path holds the text and symbols whose name holds it, best
//! first, at most [`MAX_RESULTS`].
//!
//! Matching ignores the case of ASCII letters and nothing else. A result is ranked by its name:
//! one that is exactly the text, then one that starts with it, then one that merely holds it
//! (for a file, the name is the last part of its path, and a match elsewhere in the path is a
//! match that holds). Results of one rank come by path, then line, then files before symbols,
//! then name, so the order is the same for the same store.

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::Response;
use cortex_store::{Queries, StoreError};
use serde::{Deserialize, Serialize};

use super::Ask;
use super::listing::Listing;
use super::reply::{Failure, json, respond};
use crate::map::site::Site;

/// The version of the body's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// The most results the body holds: a list a person reads to pick one, not to scroll.
const MAX_RESULTS: usize = 20;

/// The most characters the text may have: long enough for any name or path fragment a person
/// types, and short enough that a request cannot ask the store to compare a long text with every
/// name.
const MAX_QUERY_CHARS: usize = 100;

/// What the request's query holds.
#[derive(Debug, Deserialize)]
struct Params {
    /// The text to find.
    q: String,
}

/// The body.
#[derive(Debug, Serialize)]
struct Found {
    /// The shape's version.
    version: u8,
    /// The text that was searched for, as given.
    query: String,
    /// The results, best first.
    results: Vec<Hit>,
}

/// One result.
#[derive(Debug, Clone, Serialize)]
struct Hit {
    /// What was found: a `file` or a `symbol`.
    kind: &'static str,
    /// The id of the file, or of the file the symbol is in.
    file: usize,
    /// That file's path, so that a page reading an older graph can check the id.
    path: String,
    /// The file's name, or the symbol's.
    name: String,
    /// The line a symbol starts at; none for a file.
    line: Option<u32>,
}

/// A result with what orders it.
struct Ranked {
    /// How well the name matches: 0 exactly, 1 as a start, 2 as a part.
    rank: u8,
    /// What is found.
    result: Hit,
}

/// The search the request asks for, as JSON; 400 for a text of no characters or of more than
/// [`MAX_QUERY_CHARS`], or no text at all.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    respond(site, "searching the map", || {
        if ask.tail.is_some() {
            return Err(Failure::Refused(StatusCode::NOT_FOUND));
        }
        let Ok(Query(Params { q })) = Query::<Params>::try_from_uri(&ask.uri) else {
            return Err(Failure::Refused(StatusCode::BAD_REQUEST));
        };
        let length = q.chars().count();
        if length == 0 || length > MAX_QUERY_CHARS {
            return Err(Failure::Refused(StatusCode::BAD_REQUEST));
        }
        let listing = site.listing()?;
        let reader = site.source().open()?;
        let results = reader.consistently(|queries| search(queries, &listing, &q))?;
        json(&Found {
            version: VERSION,
            query: q,
            results,
        })
    })
}

/// How well `name` matches the lower-cased text `needle`: 0 when it is the text, 1 when it starts
/// with it, 2 otherwise (the caller knows it holds it).
fn rank_of(name: &str, needle: &str) -> u8 {
    let name = name.to_ascii_lowercase();
    if name == needle {
        0
    } else if name.starts_with(needle) {
        1
    } else {
        2
    }
}

/// The best [`MAX_RESULTS`] results for `text`, read through `queries`.
fn search(queries: Queries<'_>, listing: &Listing, text: &str) -> Result<Vec<Hit>, StoreError> {
    let needle = text.to_ascii_lowercase();
    let mut ranked = Vec::new();
    for (id, file) in listing.files().iter().enumerate() {
        let path = file.path.as_str();
        if !path.to_ascii_lowercase().contains(&needle) {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        ranked.push(Ranked {
            rank: rank_of(name, &needle),
            result: Hit {
                kind: "file",
                file: id,
                path: path.to_owned(),
                name: name.to_owned(),
                line: None,
            },
        });
    }
    for (name, path, line) in queries.symbols_containing(&needle, MAX_RESULTS)? {
        let Some(file) = listing.id_of_path(&path) else {
            continue;
        };
        ranked.push(Ranked {
            rank: rank_of(&name, &needle),
            result: Hit {
                kind: "symbol",
                file,
                path: path.as_str().to_owned(),
                name,
                line: Some(line),
            },
        });
    }
    // INVARIANT: deterministic (the key orders every result completely, so no two results tie)
    ranked.sort_by(|a, b| key(a).cmp(&key(b)));
    ranked.dedup_by(|a, b| key(a) == key(b));
    ranked.truncate(MAX_RESULTS);
    Ok(ranked.into_iter().map(|ranked| ranked.result).collect())
}

/// What orders a result: its rank, path, line (a file has none, and comes first), kind (a file
/// before a symbol) and name.
fn key(ranked: &Ranked) -> (u8, &str, u32, u8, &str) {
    (
        ranked.rank,
        &ranked.result.path,
        ranked.result.line.unwrap_or(0),
        u8::from(ranked.result.kind == "symbol"),
        &ranked.result.name,
    )
}
