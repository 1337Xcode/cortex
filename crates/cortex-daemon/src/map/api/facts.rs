//! `api/facts/<number>/confirm` and `api/facts/<number>/retire`, sent as a POST by the map's own
//! page: the person's word on a fact, as `cortex facts confirm` and `cortex facts retire` give
//! it. The map never writes the store: the word is handed to the daemon's writer, which carries it
//! out in a transaction of its own and answers once it is committed, the same job the command
//! makes.
//!
//! The guard lets a POST through only for these paths and only with the map's own `Origin`. The
//! answer is JSON: the fact's number and its status now; or 409 with the writer's refusal
//! ("there is no fact 7"); or 500 when the store failed to write it, saying so without the store's
//! own words; or 503 when the daemon could not take the word in time, which changed nothing. The
//! page's `Referrer-Policy` is `same-origin`, under which the Fetch standard sends a same-origin
//! POST with the page's own `Origin` in every mode; under `no-referrer` it would send
//! `Origin: null` in any mode but `cors`, which the guard refuses.

use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use cortex_core::ipc::{FactsAnswer, FactsAsk, ResponseBody};
use serde::Serialize;

use crate::asked::{ASK_START_WITHIN, ASK_WRITE_WITHIN, Ask, NotDone as Undone};
use crate::map::site::Site;

/// The version of the body's shape; nothing has consumed another.
const VERSION: u8 = 1;

/// The most digits a fact's number is written with: more could not be a row's number.
const MAX_NUMBER_DIGITS: usize = 18;

/// What the writer did.
#[derive(Debug, Serialize)]
struct Done {
    /// The shape's version.
    version: u8,
    /// The fact's number.
    id: i64,
    /// Its status now: `active` or `retired`.
    status: &'static str,
}

/// Why nothing was done.
#[derive(Debug, Serialize)]
struct NotDone {
    /// The shape's version.
    version: u8,
    /// The reason, in the daemon's words: the fact's number and status, never its text.
    error: String,
}

/// The answer to a POST of `rest`, the path after `api/facts/`: the word carried out, or 404 for a
/// path that names no word on a fact.
pub(crate) async fn write(site: Arc<Site>, rest: &str) -> Response {
    let Some(ask) = ask_of(rest) else {
        return site.empty(StatusCode::NOT_FOUND);
    };
    let source = site.source();
    source.note_in_use();
    // INVARIANT: single-writer (the map hands the word to the daemon's writer, which alone writes
    // the store; the map itself only ever reads it)
    let Some(answer) = source.ask_writer(Ask::Facts(ask)) else {
        return not_done(
            &site,
            StatusCode::SERVICE_UNAVAILABLE,
            "the daemon is stopping, or too many requests wait: try again",
        );
    };
    let within = ASK_START_WITHIN + ASK_WRITE_WITHIN;
    let waited = tokio::task::spawn_blocking(move || answer.recv_timeout(within)).await;
    match waited {
        Ok(Ok(Ok(ResponseBody::Facts(FactsAnswer::Confirmed { id })))) => done(&site, id, "active"),
        Ok(Ok(Ok(ResponseBody::Facts(FactsAnswer::Retired { id })))) => done(&site, id, "retired"),
        Ok(Ok(Ok(ResponseBody::Error { message }) | Err(Undone::Refused(message)))) => {
            not_done(&site, StatusCode::CONFLICT, &message)
        }
        Ok(Ok(Err(Undone::Failed))) => not_done(
            &site,
            StatusCode::INTERNAL_SERVER_ERROR,
            "the store failed to write it, which changed nothing: the daemon's status lists the error",
        ),
        Ok(Ok(Ok(_))) => not_done(
            &site,
            StatusCode::INTERNAL_SERVER_ERROR,
            "the daemon gave another answer",
        ),
        Ok(Err(RecvTimeoutError::Disconnected)) => not_done(
            &site,
            StatusCode::SERVICE_UNAVAILABLE,
            "the daemon was busy and did not take the word, which changed nothing: try again",
        ),
        Ok(Err(RecvTimeoutError::Timeout)) | Err(_) => not_done(
            &site,
            StatusCode::SERVICE_UNAVAILABLE,
            "the daemon did not answer in time: reload to see whether it was done",
        ),
    }
}

/// The word `rest` names: `<number>/confirm` or `<number>/retire`, the number digits only.
fn ask_of(rest: &str) -> Option<FactsAsk> {
    let (number, word) = rest.split_once('/')?;
    if number.is_empty()
        || number.len() > MAX_NUMBER_DIGITS
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let id: i64 = number.parse().ok()?;
    match word {
        "confirm" => Some(FactsAsk::Confirm { id }),
        "retire" => Some(FactsAsk::Retire { id }),
        _ => None,
    }
}

/// A 200 saying fact `id` is now `status`.
fn done(site: &Site, id: i64, status: &'static str) -> Response {
    json(
        site,
        StatusCode::OK,
        &Done {
            version: VERSION,
            id,
            status,
        },
    )
}

/// An answer of `status` saying why nothing was done.
fn not_done(site: &Site, status: StatusCode, why: &str) -> Response {
    json(
        site,
        status,
        &NotDone {
            version: VERSION,
            error: why.to_owned(),
        },
    )
}

/// `value` as JSON with `status`; a 500 with an empty body in the case serde cannot write it,
/// which a number and a few words never make happen.
fn json<T: Serialize>(site: &Site, status: StatusCode, value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(bytes) => site.reply(status, "application/json", Body::from(bytes)),
        Err(error) => {
            tracing::warn!(error = %error, "the answer to a fact's word was not written");
            site.empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words a path names: digits then `confirm` or `retire`; anything else names none.
    #[test]
    fn a_path_names_a_word_on_a_numbered_fact() {
        assert_eq!(ask_of("3/confirm"), Some(FactsAsk::Confirm { id: 3 }));
        assert_eq!(ask_of("12/retire"), Some(FactsAsk::Retire { id: 12 }));
        for refused in [
            "3",
            "3/edit",
            "/confirm",
            "-3/confirm",
            "+3/confirm",
            "3/confirm/x",
            "a/retire",
        ] {
            assert_eq!(ask_of(refused), None, "{refused}");
        }
        assert_eq!(ask_of(&format!("{}/confirm", "9".repeat(19))), None);
    }
}
