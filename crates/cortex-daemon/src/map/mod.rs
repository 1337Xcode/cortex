//! The map server: the repository's map, served over HTTP on 127.0.0.1 to the user's browser.
//!
//! Nothing listens until the daemon is first asked for the map's address. Then it binds
//! 127.0.0.1 at the port `ui.port` names (0 asks the system for a free one), makes the access
//! token, and serves from then on; later asks get the same address. The listener is a task of
//! the daemon's runtime. [`Map::stop`], when the daemon stops, closes the listener, so the port
//! refuses connections from then on, and ends every open connection the next time the server waits
//! on it. A connection also ends once no byte has gone either way for [`IDLE_LIMIT`], and at most
//! [`MAX_CONNECTIONS`] are open at once. Every request passes the guard ([`guard`]) before
//! anything is served; [`site`] says what is served and with which headers. The map reads what
//! the daemon holds and changes nothing.

#![warn(clippy::pedantic)]

mod api;
mod assets;
mod bounded;
mod connections;
mod guard;
mod settings;
mod site;
mod source;
#[cfg(test)]
mod table;
mod token;

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::Router;
use cortex_core::ipc::{MapAnswer, ResponseBody};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use self::connections::Capped;
pub(crate) use self::settings::MapSettings;
use self::site::Site;
pub(crate) use self::source::Source;
use self::token::Token;

/// How long an ask for the address waits for the daemon to have read its configuration, which it
/// does as soon as it starts listening; an ask that comes sooner than that waits, rather than
/// fail, so that `cortex map` can start the daemon and ask at once.
const CONFIGURED_WITHIN: Duration = Duration::from_secs(10);

/// The most connections the map holds at once. A browser keeps about six to one site; the rest is
/// room for a second tab and scripts that check the map, and the cap keeps a page that opens
/// connections without end from exhausting the daemon.
const MAX_CONNECTIONS: usize = 64;

/// How long a connection may go with no byte either way before it is closed, giving its slot
/// back: long enough for a page left open between two requests (a browser opens a new connection
/// when it finds one closed), and short enough that connections opened and left silent cannot
/// keep the user's own browser out for long. A web socket stays open by sending pings well within
/// it.
const IDLE_LIMIT: Duration = Duration::from_secs(30);

/// The map server: unconfigured, configured, or serving.
#[derive(Debug)]
pub(crate) struct Map {
    /// The port `ui.port` names and what the map reads, once the daemon has read its
    /// configuration.
    configured: watch::Sender<Option<Configured>>,
    /// The map as served, once it is.
    serving: Mutex<Option<Serving>>,
}

/// What the daemon's configuration sets for the map: the port to serve on and what it reads.
#[derive(Debug, Clone)]
struct Configured {
    /// The port `ui.port` names.
    port: u16,
    /// What the endpoints read.
    source: Arc<Source>,
}

/// The map as served.
struct Serving {
    /// Its address, the token included.
    url: String,
    /// Whether this build holds the map's files.
    has_assets: bool,
    /// The task accepting its connections.
    task: JoinHandle<()>,
    /// Set to true to end its open connections.
    stop: watch::Sender<bool>,
}

impl std::fmt::Debug for Serving {
    /// Never the address: it holds the token, and a debug line could reach a log.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Serving")
            .field("has_assets", &self.has_assets)
            .finish_non_exhaustive()
    }
}

impl Default for Map {
    /// A map whose port is not known yet, not served.
    fn default() -> Self {
        Self {
            configured: watch::Sender::new(None),
            serving: Mutex::new(None),
        }
    }
}

impl Map {
    /// Sets the port to serve on, from `ui.port`, and what the map reads, once the daemon has read
    /// its configuration.
    pub(crate) fn configure(&self, port: u16, source: Source) {
        self.configured.send_replace(Some(Configured {
            port,
            source: Arc::new(source),
        }));
    }

    /// The answer to an ask for the map's address: the address, the map served from now on if it
    /// was not yet; why it cannot be served; or, when the daemon has not read its configuration
    /// within [`CONFIGURED_WITHIN`], that it is still starting. Called inside the daemon's runtime.
    pub(crate) async fn answer(&self) -> ResponseBody {
        let mut watching = self.configured.subscribe();
        let waited =
            tokio::time::timeout(CONFIGURED_WITHIN, watching.wait_for(Option::is_some)).await;
        let configured = match waited {
            Ok(Ok(configured)) => configured.clone(),
            Ok(Err(_)) | Err(_) => None,
        };
        let Some(configured) = configured else {
            return ResponseBody::Error {
                message: "the daemon is still starting: try again in a moment".to_owned(),
            };
        };
        self.serve(&configured)
    }

    /// The map's address, served as `configured` says from now on if it was not yet.
    fn serve(&self, configured: &Configured) -> ResponseBody {
        let mut serving = self.serving();
        if let Some(served) = serving.as_ref() {
            return served.answer();
        }
        let port = configured.port;
        match start(port, &configured.source) {
            Ok(served) => {
                let answer = served.answer();
                *serving = Some(served);
                answer
            }
            Err(Failed::Bind(error)) => ResponseBody::Map(MapAnswer::BindFailed {
                port,
                cause: error.to_string(),
            }),
            Err(Failed::Other(message)) => ResponseBody::Error { message },
        }
    }

    /// Stops serving the map, if it was: the task accepting connections is cancelled, which
    /// closes the listener, and every open connection ends the next time the server waits on it.
    pub(crate) fn stop(&self) {
        if let Some(served) = self.serving().take() {
            served.stop.send_replace(true);
            served.task.abort();
        }
    }

    /// The map as served, locked. A panic while it was held leaves it whole (it is set in one
    /// assignment), so a poisoned lock is taken as it is.
    fn serving(&self) -> MutexGuard<'_, Option<Serving>> {
        self.serving.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Serving {
    /// The answer that gives this map's address.
    fn answer(&self) -> ResponseBody {
        ResponseBody::Map(MapAnswer::Serving {
            url: self.url.clone(),
            has_assets: self.has_assets,
        })
    }
}

/// Why the map could not be served.
#[derive(Debug)]
enum Failed {
    /// The listener could not be opened on 127.0.0.1.
    Bind(std::io::Error),
    /// Anything else, in words the asker reads.
    Other(String),
}

/// Binds 127.0.0.1 at `port` (0 for a free one), makes the token and starts serving the map,
/// whose endpoints read `source`. Called inside the daemon's runtime.
fn start(port: u16, source: &Arc<Source>) -> Result<Serving, Failed> {
    // INVARIANT: local-only (the map listens on the loopback address alone, never on another
    // interface, whatever the configuration says)
    let bound = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(Failed::Bind)?;
    bound.set_nonblocking(true).map_err(Failed::Bind)?;
    let listener = tokio::net::TcpListener::from_std(bound).map_err(Failed::Bind)?;
    let port = listener.local_addr().map_err(Failed::Bind)?.port();
    let token = Token::generate().map_err(|error| {
        Failed::Other(format!(
            "the system gave no random bytes for the map's token: {error}"
        ))
    })?;
    let url = format!("http://127.0.0.1:{port}/{}/", token.as_str());
    let site = Site::new(port, token, assets::ASSETS, Arc::clone(source))
        .map_err(|error| Failed::Other(format!("the map's headers could not be made: {error}")))?;
    let has_assets = site.has_assets();
    let router = Router::new()
        .fallback(site::respond)
        .with_state(Arc::new(site));
    let (stop, stopped) = watch::channel(false);
    // INVARIANT: budgeted (at most MAX_CONNECTIONS connections at once, each ended after
    // IDLE_LIMIT without a byte either way)
    let capped = Capped::new(listener, MAX_CONNECTIONS, IDLE_LIMIT, stopped);
    let task = tokio::spawn(async move {
        if let Err(error) = axum::serve(capped, router).await {
            tracing::warn!(error = %error, "the map stopped serving");
        }
    });
    tracing::info!(port, "the map is served");
    Ok(Serving {
        url,
        has_assets,
        task,
        stop,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::time::Instant;

    use super::*;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// How long the test waits for a byte or a condition before it fails.
    const WAIT: Duration = Duration::from_secs(10);

    /// A source over a folder that holds no store, which these tests never read.
    fn source() -> Source {
        Source::new(
            (std::path::PathBuf::from("no-store-here"), 0),
            cortex_retrieve::GraphSettings::default(),
            (Arc::default(), Arc::default()),
            MapSettings::of(&cortex_core::Config::default()),
        )
    }

    /// Asked on a free port, the map is served at an address whose health answers 200 on a
    /// connection kept open. Once stopped, that connection is closed and the port refuses new
    /// ones. The state's debug line never shows the address.
    #[test]
    fn a_stop_closes_the_port_and_the_open_connections() -> TestResult {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let map = Map::default();
        map.configure(0, source());
        let ResponseBody::Map(MapAnswer::Serving { url, .. }) = runtime.block_on(map.answer())
        else {
            return Err("the map was not served".into());
        };
        let debug = format!("{map:?}");
        assert!(
            debug.contains("Serving") && !debug.contains(&url),
            "{debug}"
        );
        let rest = url
            .strip_prefix("http://127.0.0.1:")
            .ok_or("not on 127.0.0.1")?;
        let (port, path) = rest.split_once('/').ok_or("no path")?;
        let port: u16 = port.parse()?;
        let mut open = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        open.set_read_timeout(Some(WAIT))?;
        let head = format!("GET /{path}api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        open.write_all(head.as_bytes())?;
        let mut answer = [0u8; 12];
        open.read_exact(&mut answer)?;
        assert_eq!(&answer, b"HTTP/1.1 200");
        let mut rest_of_answer = [0u8; 4096];
        let _ = open.read(&mut rest_of_answer)?;
        map.stop();
        let ended = loop {
            match open.read(&mut rest_of_answer) {
                Ok(0) => break true,
                Ok(_) => {}
                Err(error) => {
                    break !matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    );
                }
            }
        };
        assert!(ended, "the open connection was not closed");
        let until = Instant::now() + WAIT;
        while std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok() {
            assert!(Instant::now() < until, "the port still takes connections");
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
}
