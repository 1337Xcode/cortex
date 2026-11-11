//! The folders an MCP client names as its roots: a client started by an editor often runs the
//! server from a folder that is no repository (the home folder, the editor's own), and its roots
//! say which folders the user opened. They are asked once a session, at the first call, and again
//! after the client says they changed; a client that offers no roots, or names none that is a
//! local folder, gives none, and one that does not answer within [`ROOTS_WITHIN`] is not asked
//! again in the session, so that it delays one call at most. The server then works from its
//! current folder alone.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rmcp::model::{ClientResult, ServerRequest};
use rmcp::service::PeerRequestOptions;
use rmcp::{Peer, RoleServer, ServiceError};

/// How long the client has to name its roots. Past it the request is cancelled.
const ROOTS_WITHIN: Duration = Duration::from_secs(2);

/// What a session knows of its client's roots.
#[derive(Debug, Default)]
pub(crate) struct Roots {
    /// What is known so far.
    known: Mutex<Known>,
}

/// What a session knows of its client's roots at one time.
#[derive(Debug, Default, Clone)]
enum Known {
    /// Nothing yet: they are asked at the next call.
    #[default]
    Unasked,
    /// These folders, in the client's order.
    Folders(Vec<PathBuf>),
    /// The client did not answer in time: none, and not asked again in the session.
    GivenUp,
}

impl Roots {
    /// The local folders the client behind `peer` names as its roots, asked when they are not
    /// known yet.
    pub(crate) async fn of(&self, peer: &Peer<RoleServer>) -> Vec<PathBuf> {
        match self.now() {
            Known::Folders(folders) => return folders,
            Known::GivenUp => return Vec::new(),
            Known::Unasked => {}
        }
        let asked = asked(peer).await;
        let folders = match &asked {
            Known::Folders(folders) => folders.clone(),
            Known::Unasked | Known::GivenUp => Vec::new(),
        };
        // Two calls at once may both ask; the later answer is kept, and either is right.
        *self.known.lock().unwrap_or_else(PoisonError::into_inner) = asked;
        folders
    }

    /// Forgets the roots, which the client said changed, so that the next call asks again; a
    /// session that gave up on the client stays given up.
    pub(crate) fn changed(&self) {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*known, Known::Folders(_)) {
            *known = Known::Unasked;
        }
    }

    /// What is known now.
    fn now(&self) -> Known {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The roots the client behind `peer` names, asked within [`ROOTS_WITHIN`]: none for a client that
/// offers no roots or answers with an error; given up on for one that does not answer in time,
/// rmcp then cancelling the request.
#[expect(
    deprecated,
    reason = "roots are deprecated by a protocol proposal, but the clients that start Cortex's \
              server from a folder that is no repository still name the user's folders this way"
)]
async fn asked(peer: &Peer<RoleServer>) -> Known {
    let offers_roots = peer
        .peer_info()
        .is_some_and(|info| info.capabilities.roots.is_some());
    if !offers_roots {
        return Known::Folders(Vec::new());
    }
    let request = ServerRequest::ListRootsRequest(rmcp::model::ListRootsRequest::default());
    let options = PeerRequestOptions::with_timeout(ROOTS_WITHIN);
    let Ok(handle) = peer.send_request_with_option(request, options).await else {
        return Known::Folders(Vec::new());
    };
    match handle.await_response().await {
        Ok(ClientResult::ListRootsResult(listed)) => Known::Folders(
            listed
                .roots
                .iter()
                .filter_map(|root| path_of(&root.uri))
                .collect(),
        ),
        Err(ServiceError::Timeout { .. }) => Known::GivenUp,
        Ok(_) | Err(_) => Known::Folders(Vec::new()),
    }
}

/// The local folder the `file:` URI `uri` names, its scheme in any case and its escapes decoded;
/// none for another scheme, a host other than this machine, an escape that is not two
/// hexadecimal digits, text that is not UTF-8 once decoded, or a path that is not absolute.
///
/// ```text
/// file:///home/ann/my%20repo     ->  /home/ann/my repo
/// file:///C:/Users/ann/repo      ->  C:/Users/ann/repo (on Windows)
/// file:///c%3A/Users/ann/repo    ->  c:/Users/ann/repo (on Windows, as VS Code writes it)
/// ```
pub(crate) fn path_of(uri: &str) -> Option<PathBuf> {
    let scheme = uri.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = uri.get(7..)?;
    let (host, path) = rest.split_at(rest.find('/')?);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    let decoded = decoded(path)?;
    // On Windows the path after the host is `/C:/...`: the drive comes first.
    let decoded = match decoded.strip_prefix('/') {
        Some(drive) if cfg!(windows) && drive.as_bytes().get(1) == Some(&b':') => drive.to_owned(),
        _ => decoded,
    };
    let path = PathBuf::from(decoded);
    path.is_absolute().then_some(path)
}

/// `text` with each `%XX` escape replaced by its byte; none when an escape is not `%` and two
/// hexadecimal digits, or the bytes are not UTF-8.
fn decoded(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        if byte == b'%' {
            let digits = after.get(..2)?;
            // `from_str_radix` would also take a sign, as in `%+f`.
            if !digits.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            let digits = std::str::from_utf8(digits).ok()?;
            bytes.push(u8::from_str_radix(digits, 16).ok()?);
            rest = after.get(2..)?;
        } else {
            bytes.push(byte);
            rest = after;
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File URIs of this system's folders, typed by hand: escapes decoded, the scheme in any case,
    /// `localhost` taken as this machine, and on Windows VS Code's form with the drive's colon
    /// escaped.
    #[test]
    fn a_file_uri_names_a_local_folder() {
        let cases: &[(&str, &str)] = if cfg!(windows) {
            &[
                ("file:///C:/Users/ann/my%20repo", "C:/Users/ann/my repo"),
                ("file://localhost/D:/work", "D:/work"),
                ("FILE:///C:/work", "C:/work"),
                ("file:///c%3A/Users/ann/repo", "c:/Users/ann/repo"),
            ]
        } else {
            &[
                ("file:///home/ann/my%20repo", "/home/ann/my repo"),
                ("file://localhost/srv/r%C3%A9po", "/srv/r\u{e9}po"),
                ("FILE:///srv/work", "/srv/work"),
            ]
        };
        for (uri, folder) in cases {
            assert_eq!(path_of(uri), Some(PathBuf::from(folder)), "{uri}");
        }
    }

    /// Anything that is not a local folder names none, each built on a path that is absolute
    /// here, so that only what is wrong with it refuses it: another scheme, another host, a
    /// malformed escape, a signed escape, bytes that are not UTF-8, no path; and, on Windows, a
    /// path with no drive, which is not absolute there.
    #[test]
    fn anything_else_names_no_folder() {
        let absolute = if cfg!(windows) { "/C:/work" } else { "/work" };
        assert!(path_of(&format!("file://{absolute}")).is_some());
        let mut refused = vec![
            format!("https://example.test{absolute}"),
            format!("file://server{absolute}"),
            format!("file://{absolute}/%zz"),
            format!("file://{absolute}/%"),
            format!("file://{absolute}/%+f"),
            format!("file://{absolute}/%ff"),
            "file://".to_owned(),
        ];
        if cfg!(windows) {
            refused.push("file:///work".to_owned());
        }
        for uri in &refused {
            assert_eq!(path_of(uri), None, "{uri}");
        }
    }
}
