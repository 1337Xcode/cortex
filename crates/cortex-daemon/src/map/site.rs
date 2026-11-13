//! What the map answers each request with: the guard's refusal, or, for a request it lets
//! through, the API's answer, an embedded file, the page that says the build has no map, or 404.
//!
//! Every answer, refusals included, carries the same headers: `Cache-Control: no-store`, since
//! the map shows the repository as it is now and its address holds a key;
//! `X-Content-Type-Options: nosniff`, so a browser never runs a file as a type other than the one
//! given; `Referrer-Policy: same-origin`, so the address and its token never leave the map's own
//! origin in a `Referer`, while a same-origin POST still carries the page's `Origin`; and a
//! `Content-Security-Policy` letting a page load and connect to its own origin only (a web socket
//! to the map's own port included) and be framed by nothing. None carries a CORS header.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::InvalidHeaderValue;
use axum::http::header::{
    ALLOW, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST, HeaderName, LOCATION,
    ORIGIN, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use cortex_store::StoreError;

use super::api::{self, Listing, ListingCache};
use super::assets::{Asset, WITHOUT_MAP, find};
use super::guard::{self, Asked, Seen, Verdict, admit};
use super::live;
use super::source::Source;
use super::token::Token;

/// The content type of an HTML page.
const HTML: &str = "text/html; charset=utf-8";

/// The map as served: its port, its token, its embedded files, what its API reads and its page's
/// security policy.
#[derive(Debug)]
pub(crate) struct Site {
    /// The port it listens on, on 127.0.0.1.
    port: u16,
    /// Its access token.
    token: Token,
    /// Its embedded files, sorted by path.
    assets: &'static [Asset],
    /// What the API reads.
    source: Arc<Source>,
    /// The listing of files the API shares.
    listing: ListingCache,
    /// The `Content-Security-Policy` its answers carry, which names its port.
    policy: HeaderValue,
}

impl Site {
    /// The map served on 127.0.0.1 at `port` with `token`, serving the files of `assets` and an API
    /// that reads `source`.
    ///
    /// # Errors
    ///
    /// The security policy naming the port is not a header value, which a port never makes it.
    pub(crate) fn new(
        port: u16,
        token: Token,
        assets: &'static [Asset],
        source: Arc<Source>,
    ) -> Result<Self, InvalidHeaderValue> {
        let policy = format!(
            "default-src 'self'; connect-src 'self' ws://127.0.0.1:{port} ws://localhost:{port}; \
             base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'"
        );
        Ok(Self {
            port,
            token,
            assets,
            source,
            listing: ListingCache::default(),
            policy: HeaderValue::from_str(&policy)?,
        })
    }

    /// Whether the site serves any embedded file.
    pub(crate) fn has_assets(&self) -> bool {
        !self.assets.is_empty()
    }

    /// What the API reads.
    pub(super) fn source(&self) -> &Source {
        &self.source
    }

    /// The listing of the files the API shares, built again only after the store changed.
    ///
    /// # Errors
    ///
    /// The store cannot be opened or read.
    pub(super) fn listing(&self) -> Result<Arc<Listing>, StoreError> {
        self.listing.get(&self.source)
    }

    /// How many listings the site built. For tests.
    #[cfg(test)]
    pub(super) fn listings_built(&self) -> u64 {
        self.listing.builds()
    }

    /// The answer to the request whose head is `parts`; a web socket's opening takes the upgrade
    /// out of `parts`.
    pub(crate) async fn answer(self: &Arc<Self>, parts: &mut Parts) -> Response {
        let headers = &parts.headers;
        let asked = Asked {
            method: &parts.method,
            host: seen(headers, &HOST),
            origin: seen(headers, &ORIGIN),
            path: parts.uri.path(),
        };
        match admit(&asked, self.port, &self.token) {
            Verdict::Refuse(status) => self.empty(status),
            Verdict::AddSlash => self.add_slash(),
            Verdict::Serve(rest) => {
                // The path is copied so that a web socket's opening can take its upgrade out of
                // `parts`.
                let rest = rest.to_owned();
                self.serve(&rest, parts).await
            }
        }
    }

    /// The answer to a request the guard let through for `rest`, the path after the token, whose
    /// head is `parts`.
    async fn serve(self: &Arc<Self>, rest: &str, parts: &mut Parts) -> Response {
        if parts.method == axum::http::Method::POST {
            // The guard lets a POST through only for a write the map takes.
            return match rest.strip_prefix(guard::WRITES) {
                Some(write) => api::write_fact(Arc::clone(self), write).await,
                None => self.empty(StatusCode::METHOD_NOT_ALLOWED),
            };
        }
        if rest == live::PATH {
            let pacing = self.source.pacing();
            return live::answer(Arc::clone(self), parts, pacing).await;
        }
        if let Some(endpoint) = rest.strip_prefix(api::PREFIX) {
            return api::answer(Arc::clone(self), endpoint, parts.uri.clone()).await;
        }
        if rest.is_empty() && self.assets.is_empty() {
            return self.reply(StatusCode::OK, HTML, Body::from(WITHOUT_MAP));
        }
        match find(self.assets, rest) {
            Some(asset) => self.reply(StatusCode::OK, asset.content_type, Body::from(asset.bytes)),
            None => self.empty(StatusCode::NOT_FOUND),
        }
    }

    /// The answer that sends a request for the token alone on to the map's address, which ends
    /// with a slash.
    fn add_slash(&self) -> Response {
        let mut response = self.empty(StatusCode::PERMANENT_REDIRECT);
        match HeaderValue::try_from(format!("/{}/", self.token.as_str())) {
            Ok(location) => {
                response.headers_mut().insert(LOCATION, location);
                response
            }
            // A token is hexadecimal digits, which a header always holds.
            Err(_) => self.empty(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    /// An answer of `status` holding `body` of `content_type`, with the headers every answer
    /// carries.
    pub(crate) fn reply(
        &self,
        status: StatusCode,
        content_type: &'static str,
        body: Body,
    ) -> Response {
        let mut response = self.empty(status);
        *response.body_mut() = body;
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        response
    }

    /// An answer of `status` with an empty body and the headers every answer carries; one of 405
    /// also says GET is the method taken.
    pub(crate) fn empty(&self, status: StatusCode) -> Response {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = status;
        let headers = response.headers_mut();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        // `same-origin`, not `no-referrer`: the token in the address still goes, as a `Referer`,
        // only to the daemon that gave it, and the Fetch standard then sends the page's own
        // `Origin` with a same-origin POST in every mode (under `no-referrer`, `null` in any mode
        // but `cors`), which the guard asks of a fact's word.
        headers.insert(REFERRER_POLICY, HeaderValue::from_static("same-origin"));
        headers.insert(CONTENT_SECURITY_POLICY, self.policy.clone());
        if status == StatusCode::METHOD_NOT_ALLOWED {
            headers.insert(ALLOW, HeaderValue::from_static("GET"));
        }
        response
    }
}

/// Answers `request` for the map `site`: the handler every request reaches.
pub(crate) async fn respond(State(site): State<Arc<Site>>, request: Request) -> Response {
    let (mut parts, _body) = request.into_parts();
    site.answer(&mut parts).await
}

/// The header `name` of `headers` as the guard sees it.
fn seen<'a>(headers: &'a HeaderMap, name: &HeaderName) -> Seen<'a> {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (None, _) => Seen::Absent,
        (Some(value), None) => value.to_str().map_or(Seen::Unreadable, Seen::Once),
        (Some(_), Some(_)) => Seen::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Method;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// The port the map is served on in these tests.
    const PORT: u16 = 8123;

    /// A table of two files, sorted by path as the build script writes it.
    static TWO: [Asset; 2] = [
        Asset {
            path: "assets/map.js",
            content_type: "text/javascript; charset=utf-8",
            bytes: b"run()",
        },
        Asset {
            path: "index.html",
            content_type: "text/html; charset=utf-8",
            bytes: b"<p>map</p>",
        },
    ];

    /// What an answer gave: its status, its content type and its body.
    type Given = (u16, Option<String>, Vec<u8>);

    /// A source over a folder that holds no store: the API's data answers are not read here.
    fn source() -> Arc<Source> {
        source_told_by(Arc::default())
    }

    /// A source like [`source`] whose first index is the one `inbox` notes.
    fn source_told_by(inbox: Arc<crate::inbox::Inbox>) -> Arc<Source> {
        Arc::new(Source::new(
            (std::path::PathBuf::from("no-store-here"), 0),
            cortex_retrieve::GraphSettings::default(),
            (Arc::default(), Arc::default(), inbox),
            crate::map::MapSettings::of(
                &cortex_core::Config::default(),
                std::path::Path::new("no-store-here"),
            ),
        ))
    }

    /// The answer `site` gives a GET of `path` from its own origin, with any header in `extra`.
    fn get(
        site: &Arc<Site>,
        method: &Method,
        path: &str,
        extra: &[(&str, &str)],
    ) -> Result<(Response, Given), Box<dyn std::error::Error>> {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8123");
        for (name, value) in extra {
            request = request.header(*name, *value);
        }
        let (mut head, ()) = request.body(())?.into_parts();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let response = runtime.block_on(site.answer(&mut head));
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().map(str::to_owned))
            .transpose()?;
        let (parts, body) = response.into_parts();
        let bytes = runtime
            .block_on(axum::body::to_bytes(body, 1 << 20))?
            .to_vec();
        Ok((
            Response::from_parts(parts, Body::empty()),
            (status, content_type, bytes),
        ))
    }

    /// The headers every answer carries, the policy naming the site's port, and no CORS header.
    fn assert_common_headers(response: &Response) {
        let headers = response.headers();
        let value = |name: HeaderName| headers.get(name).and_then(|value| value.to_str().ok());
        assert_eq!(value(CACHE_CONTROL), Some("no-store"));
        assert_eq!(value(X_CONTENT_TYPE_OPTIONS), Some("nosniff"));
        assert_eq!(value(REFERRER_POLICY), Some("same-origin"));
        assert_eq!(
            value(CONTENT_SECURITY_POLICY),
            Some(
                "default-src 'self'; connect-src 'self' ws://127.0.0.1:8123 ws://localhost:8123; \
                 base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'"
            )
        );
        assert!(
            headers
                .keys()
                .all(|name| !name.as_str().starts_with("access-control-")),
            "{headers:?}"
        );
    }

    /// With two files embedded, each is served byte for byte with its type, the page at the
    /// address itself; the health says the files are there; anything else under the token is 404,
    /// and every answer, the refusals too, carries the common headers.
    #[test]
    fn embedded_files_are_served_byte_for_byte() -> TestResult {
        let token = Token::generate()?;
        let t = token.as_str().to_owned();
        let site = Arc::new(Site::new(PORT, token, &TWO, source())?);
        let page = (
            200,
            Some("text/html; charset=utf-8".to_owned()),
            b"<p>map</p>".to_vec(),
        );
        let script = (
            200,
            Some("text/javascript; charset=utf-8".to_owned()),
            b"run()".to_vec(),
        );
        let health = (
            200,
            Some("application/json".to_owned()),
            format!(
                "{{\"version\":\"{}\",\"has_assets\":true,\"index\":\"reading\",\"daemon\":null}}",
                env!("CARGO_PKG_VERSION")
            )
            .into_bytes(),
        );
        let missing = (404, None, Vec::new());
        for (path, expected) in [
            (format!("/{t}/"), page.clone()),
            (format!("/{t}/index.html"), page),
            (format!("/{t}/assets/map.js?v=1"), script),
            (format!("/{t}/api/health"), health),
            (format!("/{t}/api/nothing"), missing.clone()),
            (format!("/{t}/assets/other.js"), missing.clone()),
            (format!("/{t}/assets/../index.html"), missing.clone()),
            (format!("/{t}"), (308, None, Vec::new())),
            ("/api/health".to_owned(), missing),
        ] {
            let (response, given) = get(&site, &Method::GET, &path, &[])?;
            assert_eq!(given, expected, "{path}");
            assert_common_headers(&response);
        }
        Ok(())
    }

    /// The health says `reading` while the daemon's first index of the repository is under way,
    /// and `ready` once the writer notes it done.
    #[test]
    fn the_health_says_whether_the_first_index_is_done() -> TestResult {
        let inbox = Arc::new(crate::inbox::Inbox::default());
        let token = Token::generate()?;
        let path = format!("/{}/api/health", token.as_str());
        let site = Arc::new(Site::new(
            PORT,
            token,
            &TWO,
            source_told_by(Arc::clone(&inbox)),
        )?);
        let body = |site: &Arc<Site>| -> Result<String, Box<dyn std::error::Error>> {
            let (_, given) = get(site, &Method::GET, &path, &[])?;
            Ok(String::from_utf8(given.2)?)
        };
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            body(&site)?,
            format!(
                "{{\"version\":\"{version}\",\"has_assets\":true,\"index\":\"reading\",\"daemon\":null}}"
            )
        );
        inbox.mark_indexed();
        assert_eq!(
            body(&site)?,
            format!(
                "{{\"version\":\"{version}\",\"has_assets\":true,\"index\":\"ready\",\"daemon\":null}}"
            )
        );
        Ok(())
    }

    /// With no file embedded, the address serves the page that says the build has no map, the
    /// health says so, and a file's path is 404.
    #[test]
    fn a_build_without_the_map_says_so() -> TestResult {
        let token = Token::generate()?;
        let t = token.as_str().to_owned();
        let site = Arc::new(Site::new(PORT, token, &[], source())?);
        let (response, page) = get(&site, &Method::GET, &format!("/{t}/"), &[])?;
        assert_eq!(
            page,
            (
                200,
                Some("text/html; charset=utf-8".to_owned()),
                WITHOUT_MAP.as_bytes().to_vec()
            )
        );
        assert!(String::from_utf8(page.2)?.contains("made without the map's files"));
        assert_common_headers(&response);
        let (_, health) = get(&site, &Method::GET, &format!("/{t}/api/health"), &[])?;
        assert_eq!(
            health.2,
            format!(
                "{{\"version\":\"{}\",\"has_assets\":false,\"index\":\"reading\",\"daemon\":null}}",
                env!("CARGO_PKG_VERSION")
            )
            .into_bytes()
        );
        let (_, file) = get(&site, &Method::GET, &format!("/{t}/index.html"), &[])?;
        assert_eq!(file, (404, None, Vec::new()));
        Ok(())
    }

    /// The guard's refusals reach the answer: a foreign origin 403, a method other than GET 405
    /// saying GET is taken, two Host headers 403; each with an empty body and the common headers.
    /// The token without its slash is sent on to the address with it.
    #[test]
    fn refusals_are_empty_and_carry_the_headers() -> TestResult {
        let token = Token::generate()?;
        let health = format!("/{}/api/health", token.as_str());
        let site = Arc::new(Site::new(PORT, token, &TWO, source())?);
        let (foreign, given) = get(
            &site,
            &Method::GET,
            &health,
            &[("origin", "http://evil.example")],
        )?;
        assert_eq!(given, (403, None, Vec::new()));
        assert_common_headers(&foreign);
        let (post, given) = get(&site, &Method::POST, &health, &[])?;
        assert_eq!(given, (405, None, Vec::new()));
        assert_eq!(
            post.headers().get(ALLOW).map(HeaderValue::as_bytes),
            Some(&b"GET"[..])
        );
        assert_common_headers(&post);
        let (_, given) = get(&site, &Method::GET, &health, &[("host", "127.0.0.1:8123")])?;
        assert_eq!(given, (403, None, Vec::new()));
        let address = health.trim_end_matches("api/health").to_owned();
        let (moved, given) = get(&site, &Method::GET, address.trim_end_matches('/'), &[])?;
        assert_eq!(given, (308, None, Vec::new()));
        assert_eq!(
            moved.headers().get(LOCATION).map(HeaderValue::as_bytes),
            Some(address.as_bytes())
        );
        assert_common_headers(&moved);
        Ok(())
    }
}
