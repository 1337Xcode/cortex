//! `api/activity`: what agents did lately, newest first, for the map's activity feed: each event
//! an agent's hook reported (a prompt, a read, an edit, a search, a shell command, a stop and the
//! rest), with its time, its session and agent, and the file it was about when the map lists that
//! file. Edits the file watcher saw outside any agent are not agents' activity and are left out.
//!
//! The newest [`MAX_EVENTS`] events the store keeps are read; the feed is those of them an agent
//! reported. Nothing of what an event's tool read or wrote is kept, so nothing of it is shown.

use std::collections::BTreeMap;

use axum::http::StatusCode;
use axum::response::Response;
use cortex_core::{EventSource, Identifier};
use cortex_store::{EventRow, Queries, StoreError};
use serde::Serialize;

use super::Ask;
use super::listing::{Listing, Named, named};
use super::reply::{Failure, json, respond};
use crate::map::site::Site;

/// The version of the body's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// How many of the newest events are read: enough for the last few minutes of several agents'
/// work, few enough that the answer stays small.
const MAX_EVENTS: usize = 100;

/// The body.
#[derive(Debug, Serialize)]
struct Activity {
    /// The shape's version.
    version: u8,
    /// The agents' events, newest first.
    events: Vec<Happened>,
}

/// One event an agent's hook reported.
#[derive(Debug, Serialize)]
struct Happened {
    /// When, in UTC milliseconds.
    at_ms: i64,
    /// What kind of event: `prompt`, `read`, `edit`, `search`, `shell` and the rest.
    kind: &'static str,
    /// The session's id.
    session: String,
    /// Its agent's name, such as `claude`; none when the session is not stored.
    agent: Option<&'static str>,
    /// Its agent's title, such as `Claude Code`.
    title: Option<&'static str>,
    /// The file it was about, by its id in `api/graph`; none when it was about no file the map
    /// lists.
    file: Option<usize>,
    /// That file's path as the store holds it, so that a page reading an older graph can check the
    /// id; none when the event was about no file.
    path: Option<String>,
}

/// The activity of the store `site` reads, as JSON.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    respond(site, "reading the map's activity", || {
        if ask.tail.is_some() {
            return Err(Failure::Refused(StatusCode::NOT_FOUND));
        }
        let listing = site.listing()?;
        let reader = site.source().open()?;
        let activity = reader.consistently(|queries| activity_of(queries, &listing))?;
        json(&activity)
    })
}

/// The agents' events among the newest the store keeps, newest first.
fn activity_of(queries: Queries<'_>, listing: &Listing) -> Result<Activity, StoreError> {
    let rows: Vec<EventRow> = queries
        .latest_events(MAX_EVENTS)?
        .into_iter()
        .filter(|row| row.record.meta.source != Some(EventSource::Watcher))
        .filter(|row| row.record.session_id.is_some())
        .collect();
    let mut ids: Vec<Identifier> = rows
        .iter()
        .filter_map(|row| row.record.session_id.clone())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let agents: BTreeMap<Identifier, cortex_core::Agent> = queries
        .sessions_seen(&ids)?
        .into_iter()
        .map(|seen| (seen.session_id, seen.agent))
        .collect();
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(session) = row.record.session_id else {
            continue;
        };
        let agent = agents.get(&session).copied();
        let file = match row.record.node_id {
            Some(node) => named(queries, listing, node)?,
            None => Named::default(),
        };
        events.push(Happened {
            at_ms: row.record.ts_ms,
            kind: row.record.kind.name(),
            agent: agent.map(cortex_core::Agent::name),
            title: agent.map(cortex_core::Agent::title),
            file: file.id,
            path: file.path,
            session: session.as_str().to_owned(),
        });
    }
    Ok(Activity {
        version: VERSION,
        events,
    })
}
