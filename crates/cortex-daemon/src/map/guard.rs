//! Who the map answers: the one decision every request passes before anything is served.
//!
//! The map listens on 127.0.0.1, so only this machine reaches it; but on this machine a web page
//! in the user's browser can send requests to it too, and another local user can try its port.
//! [`admit`] lets a request through only when all of these hold, checked in this order:
//!
//! 1. Its `Host` header is exactly `127.0.0.1:<port>` or `localhost:<port>`, the map's own port:
//!    a page served from another name that resolves to 127.0.0.1 (DNS rebinding) sends its own
//!    name and is refused with 403. A request with no `Host`, or more than one, is refused too.
//! 2. Its `Origin` header, when there is one, is exactly `http://127.0.0.1:<port>` or
//!    `http://localhost:<port>`: a page of any other origin, `null` included, is refused with 403.
//! 3. Its path starts with `/<token>/`, or is `/<token>` alone: anything else is answered 404
//!    with nothing in it, so a request without the token learns nothing about what the map
//!    serves.
//! 4. Its method is GET: the map changes nothing, so anything else, HEAD included, is 405.
//!
//! A GET of `/<token>` alone is sent on to `/<token>/` with a 308: the page names its files and
//! the API relative to the address, which must end with a slash for them to stay under it.
//!
//! Host and Origin come first, so that a page of another site is turned away whatever it sends;
//! the token comes before the method, so that only a holder of the token learns which methods
//! the map takes. No answer carries a CORS header, so no other origin's script may read one.

use axum::http::{Method, StatusCode};

use super::token::Token;

/// A header of a request as the guard sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seen<'a> {
    /// The request has no such header.
    Absent,
    /// The request has the header once, with this text.
    Once(&'a str),
    /// The request has the header more than once, or its value is not text.
    Unreadable,
}

/// What the guard reads of a request.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Asked<'a> {
    /// Its method.
    pub(crate) method: &'a Method,
    /// Its `Host` header.
    pub(crate) host: Seen<'a>,
    /// Its `Origin` header.
    pub(crate) origin: Seen<'a>,
    /// Its path, without the query.
    pub(crate) path: &'a str,
}

/// What becomes of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict<'a> {
    /// Serve it: the path after `/<token>/`, empty for the map's page.
    Serve(&'a str),
    /// Send it on, with a 308, to `/<token>/`: it asked for `/<token>` alone.
    AddSlash,
    /// Refuse it with this status and an empty body.
    Refuse(StatusCode),
}

/// What becomes of `asked`, a request to the map served on 127.0.0.1 at `port` with `token`, by
/// the rules the module's documentation lists.
pub(crate) fn admit<'a>(asked: &Asked<'a>, port: u16, token: &Token) -> Verdict<'a> {
    let Seen::Once(host) = asked.host else {
        return Verdict::Refuse(StatusCode::FORBIDDEN);
    };
    if !is_own_host(host, port) {
        return Verdict::Refuse(StatusCode::FORBIDDEN);
    }
    match asked.origin {
        Seen::Absent => {}
        Seen::Once(origin) if is_own_origin(origin, port) => {}
        Seen::Once(_) | Seen::Unreadable => return Verdict::Refuse(StatusCode::FORBIDDEN),
    }
    let Some(verdict) = under_token(asked.path, token) else {
        return Verdict::Refuse(StatusCode::NOT_FOUND);
    };
    if asked.method != Method::GET {
        return Verdict::Refuse(StatusCode::METHOD_NOT_ALLOWED);
    }
    verdict
}

/// The names the map answers to: the address it listens on, and the name every system gives it.
const OWN_NAMES: [&str; 2] = ["127.0.0.1", "localhost"];

/// Whether `host`, a `Host` header, is one of [`OWN_NAMES`] with `port`, written exactly so.
fn is_own_host(host: &str, port: u16) -> bool {
    let Some((name, written_port)) = host.rsplit_once(':') else {
        return false;
    };
    OWN_NAMES.contains(&name) && written_port == port.to_string()
}

/// Whether `origin`, an `Origin` header, is the map's own origin under one of [`OWN_NAMES`],
/// written exactly so.
fn is_own_origin(origin: &str, port: u16) -> bool {
    origin
        .strip_prefix("http://")
        .is_some_and(|host| is_own_host(host, port))
}

/// What a GET of `path` gets once Host and Origin passed: the rest of it served when it starts
/// with `/<token>/`, sent on when it is `/<token>` alone; none for any other path.
fn under_token<'a>(path: &'a str, token: &Token) -> Option<Verdict<'a>> {
    let under = path.strip_prefix('/')?;
    let (first, verdict) = match under.split_once('/') {
        Some((first, rest)) => (first, Verdict::Serve(rest)),
        None => (under, Verdict::AddSlash),
    };
    token.matches(first).then_some(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The port the map is served on in these rows.
    const PORT: u16 = 8123;

    /// One row of the table: what it checks, the request's method, `Host`, `Origin` and path, and
    /// the path served or the status refused with.
    type Row<'a> = (
        &'a str,
        &'a Method,
        Seen<'a>,
        Seen<'a>,
        &'a str,
        Result<&'a str, u16>,
    );

    /// Every rule, one row each, with the status it gives, and the requests it lets through with
    /// the path each serves; the token alone, without its slash, is sent on with a 308.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one row per rule, written out in full, reads best as one table"
    )]
    fn each_rule_refuses_with_its_status() -> Result<(), getrandom::Error> {
        let token = Token::generate()?;
        let t = token.as_str();
        let health = format!("/{t}/api/health");
        let page = format!("/{t}/");
        let wrong_token = format!("/{}/api/health", "0".repeat(64));
        let token_alone = format!("/{t}");
        let wrong_alone = format!("/{}", "0".repeat(64));
        let (get, post, head) = (&Method::GET, &Method::POST, &Method::HEAD);
        let ours = Seen::Once("127.0.0.1:8123");
        // (what the row checks, method, Host, Origin, path, status or the path served)
        let rows: [Row<'_>; 23] = [
            ("served", get, ours, Seen::Absent, &health, Ok("api/health")),
            (
                "by its name",
                get,
                Seen::Once("localhost:8123"),
                Seen::Absent,
                &page,
                Ok(""),
            ),
            (
                "own origin",
                get,
                ours,
                Seen::Once("http://127.0.0.1:8123"),
                &health,
                Ok("api/health"),
            ),
            (
                "other own origin",
                get,
                ours,
                Seen::Once("http://localhost:8123"),
                &page,
                Ok(""),
            ),
            (
                "no Host",
                get,
                Seen::Absent,
                Seen::Absent,
                &health,
                Err(403),
            ),
            (
                "Host twice",
                get,
                Seen::Unreadable,
                Seen::Absent,
                &health,
                Err(403),
            ),
            (
                "DNS rebinding",
                get,
                Seen::Once("evil.example:8123"),
                Seen::Absent,
                &health,
                Err(403),
            ),
            (
                "other port",
                get,
                Seen::Once("127.0.0.1:8124"),
                Seen::Absent,
                &health,
                Err(403),
            ),
            (
                "no port",
                get,
                Seen::Once("127.0.0.1"),
                Seen::Absent,
                &health,
                Err(403),
            ),
            (
                "name in capitals",
                get,
                Seen::Once("LOCALHOST:8123"),
                Seen::Absent,
                &page,
                Err(403),
            ),
            (
                "all addresses",
                get,
                Seen::Once("0.0.0.0:8123"),
                Seen::Absent,
                &page,
                Err(403),
            ),
            (
                "foreign origin",
                get,
                ours,
                Seen::Once("http://evil.example"),
                &health,
                Err(403),
            ),
            (
                "null origin",
                get,
                ours,
                Seen::Once("null"),
                &health,
                Err(403),
            ),
            (
                "secure scheme",
                get,
                ours,
                Seen::Once("https://127.0.0.1:8123"),
                &health,
                Err(403),
            ),
            (
                "origin twice",
                get,
                ours,
                Seen::Unreadable,
                &health,
                Err(403),
            ),
            ("no token", get, ours, Seen::Absent, "/api/health", Err(404)),
            ("root", get, ours, Seen::Absent, "/", Err(404)),
            (
                "wrong token",
                get,
                ours,
                Seen::Absent,
                &wrong_token,
                Err(404),
            ),
            (
                "token without its slash",
                get,
                ours,
                Seen::Absent,
                &token_alone,
                Err(308),
            ),
            (
                "wrong token without a slash",
                get,
                ours,
                Seen::Absent,
                &wrong_alone,
                Err(404),
            ),
            (
                "POST to the token alone",
                post,
                ours,
                Seen::Absent,
                &token_alone,
                Err(405),
            ),
            ("POST", post, ours, Seen::Absent, &health, Err(405)),
            ("HEAD", head, ours, Seen::Absent, &health, Err(405)),
        ];
        for (name, method, host, origin, path, expected) in rows {
            let asked = Asked {
                method,
                host,
                origin,
                path,
            };
            let verdict = match admit(&asked, PORT, &token) {
                Verdict::Serve(rest) => Ok(rest),
                Verdict::AddSlash => Err(308),
                Verdict::Refuse(status) => Err(status.as_u16()),
            };
            assert_eq!(verdict, expected, "{name}");
        }
        Ok(())
    }

    /// The order the rules are checked in: a POST from another site is refused for its site,
    /// and a POST without the token is told nothing about methods.
    #[test]
    fn site_comes_before_token_and_token_before_method() -> Result<(), getrandom::Error> {
        let token = Token::generate()?;
        let health = format!("/{}/api/health", token.as_str());
        let foreign = Asked {
            method: &Method::POST,
            host: Seen::Once("evil.example:8123"),
            origin: Seen::Once("http://evil.example"),
            path: "/api/health",
        };
        assert_eq!(
            admit(&foreign, PORT, &token),
            Verdict::Refuse(StatusCode::FORBIDDEN)
        );
        let tokenless = Asked {
            host: Seen::Once("127.0.0.1:8123"),
            origin: Seen::Absent,
            ..foreign
        };
        assert_eq!(
            admit(&tokenless, PORT, &token),
            Verdict::Refuse(StatusCode::NOT_FOUND)
        );
        let holder = Asked {
            path: &health,
            ..tokenless
        };
        assert_eq!(
            admit(&holder, PORT, &token),
            Verdict::Refuse(StatusCode::METHOD_NOT_ALLOWED)
        );
        Ok(())
    }
}
