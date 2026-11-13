//! `api/health`: the daemon's version, whether this build holds the map's files, whether the
//! daemon is still reading the repository for the first time, and how the daemon is doing, so that
//! the page and a script checking the map can tell what they talk to, an empty map can say why it
//! is empty, and the health panel can show what `cortex status` shows.
//!
//! The daemon part comes from the daemon's status report as it stands at the request, not from the
//! store, so two requests give different bodies as the daemon runs on.

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use cortex_core::ipc::{CORTEX_VERSION, HookLatency, RecentError, StatusReport};
use cortex_core::one_line;
use serde::Serialize;

use super::Ask;
use super::history::{self, Reader};
use crate::map::site::Site;

/// The most errors the body names: the newest, as `cortex status` keeps a few.
const MAX_ERRORS: usize = 10;

/// The longest miner detail the body gives, in characters, as for the daemon's error messages: a
/// reason naming a long path stays readable, and the page's parser has a bound to hold to.
const MAX_DETAIL_CHARS: usize = 1_000;

/// The endpoint's answer.
#[derive(Debug, Serialize)]
struct Health {
    /// The daemon's Cortex version.
    version: &'static str,
    /// Whether this build holds the map's files.
    has_assets: bool,
    /// `reading` until the daemon's first index of the repository is done, `ready` after.
    index: &'static str,
    /// How the daemon is doing; none when it cannot say, as while it stops.
    daemon: Option<Daemon>,
}

/// How the daemon is doing, from its status report.
#[derive(Debug, Serialize)]
struct Daemon {
    /// How long it has run, in milliseconds.
    uptime_ms: u64,
    /// How long it took to handle each hook's latest requests, in hook order: its own handling,
    /// from the request's first byte to its answer, not the hook's whole run.
    hooks: Vec<HookLatency>,
    /// Its latest errors, newest first, at most [`MAX_ERRORS`].
    errors: Vec<RecentError>,
    /// Hook events dropped because too many waited, since it started.
    events_dropped: u64,
    /// Batches of events the store could not record, since it started.
    failed_batches: u64,
    /// Where the co-change miner is: `waiting`, `no_repository`, `mining`, `ready` or `failed`.
    git: &'static str,
    /// More about the miner's state in words, when there is more: one line of at most
    /// [`MAX_DETAIL_CHARS`] characters.
    git_detail: Option<String>,
    /// The graph retrieval spreads over, as last built: its nodes; none before its first build.
    graph_nodes: Option<u64>,
    /// Its links.
    graph_links: Option<u64>,
    /// When upkeep last ran, in UTC milliseconds; none before its first run.
    upkeep_at_ms: Option<i64>,
    /// Whether the history worker reads blames: `on`, `off` or `refused`.
    history: Reader,
}

/// The health of `site` as JSON; a 500 with an empty body when it cannot be written, and 404 for
/// a request that names more than the endpoint.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    if ask.tail.is_some() {
        return site.empty(StatusCode::NOT_FOUND);
    }
    let source = site.source();
    let health = Health {
        version: CORTEX_VERSION,
        has_assets: site.has_assets(),
        index: if source.is_first_index_done() {
            "ready"
        } else {
            "reading"
        },
        daemon: source.status_report().map(daemon_of),
    };
    match serde_json::to_vec(&health) {
        Ok(json) => site.reply(StatusCode::OK, "application/json", Body::from(json)),
        Err(error) => {
            tracing::warn!(error = %error, "the map's health was not written");
            site.empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// What the health panel shows of `report`.
fn daemon_of(report: StatusReport) -> Daemon {
    let git = report.git.unwrap_or_default();
    let mut errors = report.recent_errors;
    errors.reverse();
    errors.truncate(MAX_ERRORS);
    Daemon {
        uptime_ms: report.uptime_ms,
        hooks: report.latencies,
        errors,
        events_dropped: report.events_dropped,
        failed_batches: report.failed_batches,
        git: git.state.name(),
        git_detail: git.detail.map(|detail| one_line(&detail, MAX_DETAIL_CHARS)),
        graph_nodes: report.graph.as_ref().map(|graph| graph.nodes),
        graph_links: report.graph.as_ref().map(|graph| graph.links),
        upkeep_at_ms: report.upkeep.map(|upkeep| upkeep.at_ms),
        history: history::reader_from(report.history.as_ref()),
    }
}
