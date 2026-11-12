//! The map's API: the endpoints under `api/` past the token, each a file of its own and a line of
//! [`ENDPOINTS`]. Each answers JSON from what the daemon holds; none changes anything.

mod health;

use axum::response::Response;

use super::site::Site;

/// The path every endpoint's name follows, after the token.
pub(crate) const PREFIX: &str = "api/";

/// How an endpoint answers for the map it serves.
type Endpoint = fn(&Site) -> Response;

/// Every endpoint, by the name that follows [`PREFIX`].
const ENDPOINTS: [(&str, Endpoint); 1] = [("health", health::answer)];

/// The answer of the endpoint named `name` for `site`; none for a name no endpoint has.
pub(crate) fn answer(site: &Site, name: &str) -> Option<Response> {
    ENDPOINTS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, endpoint)| endpoint(site))
}
