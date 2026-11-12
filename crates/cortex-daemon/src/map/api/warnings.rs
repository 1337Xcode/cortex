//! `api/warnings`: the warnings Cortex gave agents after their edits, newest first, open and
//! resolved, for the map's warnings panel: each the kind of warning (a file that usually changes
//! with the one edited, a test of it, or a caller of a symbol whose signature changed), the file
//! the edit was about and the file the warning named, by their ids in `api/graph`, its strength,
//! and what the end of its task found: still open, followed (the named file was read or edited
//! after it) or not followed. An edit's note of where the edited code came from is no warning and
//! is left out, as `cortex receipts` leaves it out of the warnings' precision.

use axum::http::StatusCode;
use axum::response::Response;
use cortex_core::WarningKind;
use cortex_store::{NodeId, Queries, StoreError};
use serde::Serialize;

use super::Ask;
use super::listing::Listing;
use super::reply::{Failure, json, respond};
use crate::map::site::Site;

/// The version of the body's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// How many of the newest warnings are read: a few days of an agent's edits, few enough that the
/// answer stays small.
const MAX_WARNINGS: usize = 100;

/// The body.
#[derive(Debug, Serialize)]
struct Warnings {
    /// The shape's version.
    version: u8,
    /// The warnings, newest first.
    warnings: Vec<Given>,
}

/// One file a warning named.
#[derive(Debug, Serialize)]
struct Given {
    /// When, in UTC milliseconds.
    at_ms: i64,
    /// `cochange`, `test` or `call_site`.
    kind: &'static str,
    /// The file the edit was about, by its id; none when the map no longer lists it.
    source: Option<usize>,
    /// The file the warning named, by its id; none when the map no longer lists it.
    target: Option<usize>,
    /// How strong its evidence was, from 0 to 1.
    strength: f64,
    /// `open` until its task ends, then `followed` or `not_followed`.
    state: &'static str,
}

/// The warnings of the store `site` reads, as JSON.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    respond(site, "reading the map's warnings", || {
        if ask.tail.is_some() {
            return Err(Failure::Refused(StatusCode::NOT_FOUND));
        }
        let listing = site.listing()?;
        let reader = site.source().open()?;
        let warnings = reader.consistently(|queries| warnings_of(queries, &listing))?;
        json(&warnings)
    })
}

/// The newest warnings `queries` hold, origin notes left out.
fn warnings_of(queries: Queries<'_>, listing: &Listing) -> Result<Warnings, StoreError> {
    let mut warnings = Vec::new();
    for row in queries.latest_warnings(MAX_WARNINGS)? {
        if row.record.kind == WarningKind::Origin {
            continue;
        }
        warnings.push(Given {
            at_ms: row.record.at_ms,
            kind: row.record.kind.name(),
            source: file_of(queries, listing, row.record.source)?,
            target: file_of(queries, listing, row.record.target)?,
            strength: row.record.strength,
            state: match row.resolved {
                None => "open",
                Some(true) => "followed",
                Some(false) => "not_followed",
            },
        });
    }
    Ok(Warnings {
        version: VERSION,
        warnings,
    })
}

/// The id of the file `node` is, or holds when it is a symbol; none when the map lists no such
/// file.
fn file_of(
    queries: Queries<'_>,
    listing: &Listing,
    node: NodeId,
) -> Result<Option<usize>, StoreError> {
    if let Some(id) = listing.id_of_node(node) {
        return Ok(Some(id));
    }
    Ok(queries
        .node_file(node)?
        .and_then(|(_, path)| listing.id_of_path(&path)))
}
