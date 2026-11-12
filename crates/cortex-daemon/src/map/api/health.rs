//! `api/health`: the daemon's version, whether this build holds the map's files, and whether the
//! daemon is still reading the repository for the first time, so that the page and a script
//! checking the map can tell what they talk to, and an empty map can say why it is empty.

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use cortex_core::ipc::CORTEX_VERSION;
use serde::Serialize;

use super::Ask;
use crate::map::site::Site;

/// The endpoint's answer.
#[derive(Debug, Serialize)]
struct Health {
    /// The daemon's Cortex version.
    version: &'static str,
    /// Whether this build holds the map's files.
    has_assets: bool,
    /// `reading` until the daemon's first index of the repository is done, `ready` after.
    index: &'static str,
}

/// The health of `site` as JSON; a 500 with an empty body when it cannot be written, which a
/// struct of strings and a flag never makes happen, and 404 for a request that names more than
/// the endpoint.
pub(super) fn answer(site: &Site, ask: &Ask) -> Response {
    if ask.tail.is_some() {
        return site.empty(StatusCode::NOT_FOUND);
    }
    let health = Health {
        version: CORTEX_VERSION,
        has_assets: site.has_assets(),
        index: if site.source().is_first_index_done() {
            "ready"
        } else {
            "reading"
        },
    };
    match serde_json::to_vec(&health) {
        Ok(json) => site.reply(StatusCode::OK, "application/json", Body::from(json)),
        Err(error) => {
            tracing::warn!(error = %error, "the map's health was not written");
            site.empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
