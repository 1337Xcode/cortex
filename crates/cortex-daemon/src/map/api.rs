//! The map's API: the endpoints under `api/` past the token, each a file of its own and a line of
//! [`ENDPOINTS`]. Each answers JSON from what the daemon holds; none changes anything.
//!
//! An endpoint's work is blocking (it reads the store), so it runs on a thread of the runtime's
//! blocking pool and the request waits for it at most [`ANSWER_WITHIN`]. Every list an endpoint
//! gives is sorted by a rule its module states, and every number comes from the store as it stood
//! in one read, so that one store gives byte-identical bodies.

mod facts;
mod file;
mod graph;
mod health;
mod history;
mod links;
mod listing;
mod numbers;
mod receipts;
mod reply;
mod search;
mod summary;
#[cfg(test)]
mod tests;
mod told;

use std::sync::Arc;
use std::time::Duration;

use axum::http::{StatusCode, Uri};
use axum::response::Response;

use super::site::Site;

pub(super) use self::facts::write as write_fact;
pub(super) use self::listing::{Listing, ListingCache};

/// The path every endpoint's name follows, after the token.
pub(crate) const PREFIX: &str = "api/";

/// How long a request waits for an endpoint's work. The work goes on when the wait ends, as a
/// read of the store cannot be called off; the connection cap bounds how many such reads there
/// are, and the one that outlasts the wait finds its answer unwanted and drops it.
pub(super) const ANSWER_WITHIN: Duration = Duration::from_secs(10);

/// What an endpoint is asked.
#[derive(Debug)]
pub(super) struct Ask {
    /// What follows the endpoint's name and a slash, if anything does: `12` for `api/file/12`.
    pub(super) tail: Option<String>,
    /// The request's address, whose query an endpoint may read.
    pub(super) uri: Uri,
}

/// How an endpoint answers for the map it serves; it may block.
type Endpoint = fn(&Site, &Ask) -> Response;

/// Every endpoint, by the name that follows [`PREFIX`].
const ENDPOINTS: [(&str, Endpoint); 6] = [
    ("file", file::answer),
    ("graph", graph::answer),
    ("health", health::answer),
    ("receipts", receipts::answer),
    ("search", search::answer),
    ("summary", summary::answer),
];

/// The answer to a request for `endpoint`, the path after [`PREFIX`], with the address `uri`: the
/// endpoint's own, 404 for a name no endpoint has, 500 when its work failed to finish and 503
/// when it took longer than [`ANSWER_WITHIN`].
pub(crate) async fn answer(site: Arc<Site>, endpoint: &str, uri: Uri) -> Response {
    let (name, tail) = match endpoint.split_once('/') {
        Some((name, tail)) => (name, Some(tail.to_owned())),
        None => (endpoint, None),
    };
    let Some((name, run)) = ENDPOINTS.iter().copied().find(|(known, _)| *known == name) else {
        return site.empty(StatusCode::NOT_FOUND);
    };
    let ask = Ask { tail, uri };
    let on_site = Arc::clone(&site);
    // INVARIANT: budgeted (a request waits for an endpoint at most ANSWER_WITHIN, and the
    // connection cap bounds how many endpoints work at once)
    let working = tokio::task::spawn_blocking(move || run(&on_site, &ask));
    match tokio::time::timeout(ANSWER_WITHIN, working).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::warn!(endpoint = name, error = %error, "a map endpoint did not finish");
            site.empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(_) => {
            tracing::warn!(endpoint = name, "a map endpoint took too long to answer");
            site.empty(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
