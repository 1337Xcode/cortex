//! How an endpoint's work becomes an answer: JSON when it worked, an empty body and a status
//! when it was refused, and 500 with an empty body and a line in the log when it failed.
//!
//! A log line names the operation and the cause the store or serde gives, which hold row and
//! column names and never a file's content.

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use serde::Serialize;

use crate::map::site::Site;

/// Why an endpoint's work gave no body.
#[derive(Debug)]
pub(super) enum Failure {
    /// The request asked for something the endpoint refuses, with this status.
    Refused(StatusCode),
    /// Reading the store or writing the body failed, for this cause.
    Broken(String),
}

impl From<cortex_store::StoreError> for Failure {
    /// A store error is a failure of the read.
    fn from(error: cortex_store::StoreError) -> Self {
        Self::Broken(format!("{error:#}"))
    }
}

impl From<serde_json::Error> for Failure {
    /// A body that could not be written is a failure.
    fn from(error: serde_json::Error) -> Self {
        Self::Broken(error.to_string())
    }
}

/// The body of `value` as JSON.
///
/// # Errors
///
/// A value serde cannot write, which a struct of numbers and strings never is.
pub(super) fn json<T: Serialize>(value: &T) -> Result<Vec<u8>, Failure> {
    Ok(serde_json::to_vec(value)?)
}

/// The answer `work` gives for `site`: its body as JSON with a 200, a refusal's status with an
/// empty body, or, for a failure, a 500 with an empty body after a log line naming `operation`.
pub(super) fn respond(
    site: &Site,
    operation: &'static str,
    work: impl FnOnce() -> Result<Vec<u8>, Failure>,
) -> Response {
    match work() {
        Ok(body) => site.reply(StatusCode::OK, "application/json", Body::from(body)),
        Err(Failure::Refused(status)) => site.empty(status),
        Err(Failure::Broken(cause)) => {
            tracing::warn!(operation, cause = %cause, "the map's data was not read");
            site.empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
