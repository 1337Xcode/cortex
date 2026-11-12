//! `api/live`: a web socket the map's page opens to see where agent sessions work, as it happens.
//!
//! The daemon sends text frames, each the whole live state ([`state::frame_of`]), never a delta:
//! one as soon as the socket opens, then one whenever the store may hold new positions and what
//! it holds differs from the last frame, at most one per [`Pacing::gap`] (four a second). It reads
//! the state again every [`Pacing::refresh`] too, as sessions stop being live with time alone, and
//! pings the page then, which keeps the connection from the map's idle close and, while the page
//! is open, puts off the daemon's own idle stop. The page sends nothing; a text or binary message
//! from it is ignored, and one over [`MAX_MESSAGE_BYTES`] ends the socket.
//!
//! The socket passes the same guard as every request (the token, `Host` and `Origin`), counts
//! against the same cap of connections, and ends when the map stops.

mod pacer;
mod state;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::FromRequestParts;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::Response;
use tokio::time::{Instant, MissedTickBehavior};

use self::pacer::Pacer;
use super::api::ANSWER_WITHIN;
use super::site::Site;

/// The endpoint's path after the token.
pub(super) const PATH: &str = "api/live";

/// The largest message read from the page. It sends nothing, so anything larger is not the map's
/// own page talking.
const MAX_MESSAGE_BYTES: usize = 1024;

/// How a live socket paces its work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pacing {
    /// The least time between two frames: 250 ms, four a second, which a person sees as live.
    pub(crate) gap: Duration,
    /// How often the socket looks whether anything changed, cheaply, without reading the store.
    pub(crate) poll: Duration,
    /// How often it reads the state again whatever changed, pings the page and notes the map in
    /// use: 15 s, well within the map's idle close of 30 s.
    pub(crate) refresh: Duration,
}

/// The pace the map serves at.
pub(crate) const PACING: Pacing = Pacing {
    gap: Duration::from_millis(250),
    poll: Duration::from_millis(250),
    refresh: Duration::from_secs(15),
};

/// The answer to `parts`, a request the guard let through for [`PATH`]: the socket's opening, or
/// 400 for a request that is not a web socket's opening.
pub(super) async fn answer(site: Arc<Site>, parts: &mut Parts, pacing: Pacing) -> Response {
    let Ok(upgrade) = WebSocketUpgrade::from_request_parts(parts, &()).await else {
        return site.empty(StatusCode::BAD_REQUEST);
    };
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve(site, socket, pacing))
}

/// Sends `site`'s live state on `socket` until the page closes it, it fails, or the map stops
/// (which closes the connection under it, so the next receive ends the loop).
async fn serve(site: Arc<Site>, mut socket: WebSocket, pacing: Pacing) {
    let mut pacer = Pacer::new(pacing.gap);
    let mut poll = tokio::time::interval(pacing.poll);
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut next_refresh = Instant::now() + pacing.refresh;
    let mut activity = site.source().activity();
    let mut last_sent: Option<String> = None;
    pacer.note_change();
    loop {
        // The tick's own time, not the time the task woke: a late wake must not make the gap
        // since the last frame look shorter than it is.
        let now = tokio::select! {
            received = socket.recv() => {
                if is_end(received.as_ref()) {
                    return;
                }
                continue;
            }
            tick = poll.tick() => tick,
        };
        let seen = site.source().activity();
        if seen != activity {
            activity = seen;
            pacer.note_change();
        }
        if now >= next_refresh {
            next_refresh = now + pacing.refresh;
            site.source().note_in_use();
            pacer.note_change();
            if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                return;
            }
        }
        if !pacer.take_due(now.into_std()) {
            continue;
        }
        // A frame that cannot be read now is dropped, not retried at once: an unreadable store
        // would otherwise log a warning every gap. The next change or refresh reads it again.
        let frame = match read_beside(&site, &mut socket).await {
            Read::Frame(frame) => frame,
            Read::Missed => continue,
            Read::Ended => return,
        };
        if last_sent.as_deref() == Some(frame.as_str()) {
            continue;
        }
        if socket.send(Message::text(frame.clone())).await.is_err() {
            return;
        }
        last_sent = Some(frame);
    }
}

/// Whether `received`, the socket's next message, ends it: a close, an error, or the end of the
/// connection. Anything else the page sends is ignored.
fn is_end(received: Option<&Result<Message, axum::Error>>) -> bool {
    matches!(received, Some(Ok(Message::Close(_)) | Err(_)) | None)
}

/// How a read of the live state beside the socket ended.
enum Read {
    /// The state, as JSON.
    Frame(String),
    /// The store could not be read in time; the cause is logged.
    Missed,
    /// The socket ended while the store was read.
    Ended,
}

/// Reads `site`'s live state while still receiving from `socket`, so a page that closes or a map
/// that stops is noticed during a slow read.
async fn read_beside(site: &Arc<Site>, socket: &mut WebSocket) -> Read {
    let reading = read_frame(site);
    tokio::pin!(reading);
    loop {
        tokio::select! {
            frame = &mut reading => return frame.map_or(Read::Missed, Read::Frame),
            received = socket.recv() => {
                if is_end(received.as_ref()) {
                    return Read::Ended;
                }
            }
        }
    }
}

/// The live state of `site` as JSON, read on a thread of the blocking pool; none when the store
/// cannot be read now or the read outlasts [`ANSWER_WITHIN`] (the cause logged).
async fn read_frame(site: &Arc<Site>) -> Option<String> {
    let site = Arc::clone(site);
    let read = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let listing = site.listing().map_err(|error| error.to_string())?;
        let source = site.source();
        let reader = source.open().map_err(|error| error.to_string())?;
        let frame = reader
            .consistently(|queries| {
                state::frame_of(queries, &listing, source.now_ms(), source.timing())
            })
            .map_err(|error| error.to_string())?;
        serde_json::to_string(&frame).map_err(|error| error.to_string())
    });
    // INVARIANT: budgeted (a live socket waits for a read of the store at most ANSWER_WITHIN, has
    // one read at a time, and the connection cap bounds the sockets)
    match tokio::time::timeout(ANSWER_WITHIN, read).await {
        Ok(Ok(Ok(frame))) => Some(frame),
        Ok(Ok(Err(error))) => {
            tracing::warn!(error = %error, "the map's live state was not read");
            None
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "reading the map's live state did not finish");
            None
        }
        Err(_) => {
            tracing::warn!("reading the map's live state took too long");
            None
        }
    }
}
