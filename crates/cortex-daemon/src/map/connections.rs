//! The map's listener, holding at most a fixed number of connections at once: a page in the
//! user's browser could otherwise open connections to it without end. A connection past the cap
//! is closed as soon as it is accepted, before anything is read from it. Each connection it gives
//! the server is [`Bounded`]: it ends when idle too long or when the map stops, giving its slot
//! back. [`serve_capped`] serves on it with a limit on how long a request's head may take.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::serve::Listener;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use super::bounded::Bounded;

/// A TCP listener that keeps at most as many connections open as its permits allow.
#[derive(Debug)]
pub(crate) struct Capped {
    /// The listener.
    listener: TcpListener,
    /// One permit per connection that may be open at once.
    permits: Arc<Semaphore>,
    /// How long a connection may go with no byte either way.
    idle_limit: Duration,
    /// Holds true once the map stops.
    stop: watch::Receiver<bool>,
}

impl Capped {
    /// `listener`, holding at most `max_connections` connections at once, each ended when idle
    /// for `idle_limit` or once `stop` holds true.
    pub(crate) fn new(
        listener: TcpListener,
        max_connections: usize,
        idle_limit: Duration,
        stop: watch::Receiver<bool>,
    ) -> Self {
        Self {
            listener,
            permits: Arc::new(Semaphore::new(max_connections)),
            idle_limit,
            stop,
        }
    }
}

impl Listener for Capped {
    type Io = Bounded<Held>;
    type Addr = SocketAddr;

    /// The next connection there is a permit for; the others are closed as they arrive. An error
    /// accepting one is handled as the server's TCP listener handles it.
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, address) = Listener::accept(&mut self.listener).await;
            if let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() {
                let held = Held {
                    stream,
                    _permit: permit,
                };
                let bounded = Bounded::new(held, self.idle_limit, self.stop.clone());
                return (bounded, address);
            }
            drop(stream);
        }
    }

    /// The address the listener is bound to.
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

/// Serves `router` on every connection `listener` accepts, each request's head (its request line
/// and headers) to arrive in full within `head_limit`, or the connection is closed. The idle limit
/// alone does not bound this: a byte every few seconds keeps a connection from being idle, so a
/// local process could hold every slot by dribbling heads it never finishes. The limit runs again
/// for each request a kept-alive connection makes, from when the server starts waiting for it.
/// Runs until its task is stopped.
pub(crate) async fn serve_capped(mut listener: Capped, router: Router, head_limit: Duration) {
    loop {
        let (connection, _address) = Listener::accept(&mut listener).await;
        let service = TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            let mut builder = http1::Builder::new();
            // INVARIANT: budgeted (a request's head arrives within head_limit or its connection
            // ends, giving its slot back)
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(head_limit);
            let served = builder
                .serve_connection(TokioIo::new(connection), service)
                .with_upgrades()
                .await;
            if let Err(error) = served {
                tracing::debug!(error = %error, "a map connection ended");
            }
        });
    }
}

/// An accepted connection, holding its permit until it closes.
#[derive(Debug)]
pub(crate) struct Held {
    /// The connection.
    stream: TcpStream,
    /// Its permit, given back when the connection is dropped.
    _permit: OwnedSemaphorePermit,
}

impl AsyncRead for Held {
    /// Reads from the connection.
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for Held {
    /// Writes to the connection.
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(context, bytes)
    }

    /// Flushes the connection.
    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }

    /// Shuts the connection's writing half.
    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::Ipv4Addr;
    use std::time::Instant;

    use axum::Router;

    use super::*;
    use crate::map::{HEAD_LIMIT, IDLE_LIMIT, MAX_CONNECTIONS};

    /// Result type for these tests: any failure fails the test.
    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// How long a client waits for a byte, and for a condition, before the test fails.
    const WAIT: Duration = Duration::from_secs(10);

    /// A server answering `ok` to every request on 127.0.0.1, behind a listener capped at
    /// `max_connections`, each ended when idle for `idle_limit`, its heads given the map's own
    /// limit; its port, and the sender of its stop, which ends it when dropped.
    fn serve(
        runtime: &tokio::runtime::Runtime,
        max_connections: usize,
        idle_limit: Duration,
    ) -> TestResult<(u16, watch::Sender<bool>)> {
        serve_with_head_limit(runtime, (max_connections, idle_limit), HEAD_LIMIT)
    }

    /// [`serve`], each request's head to arrive within `head_limit`.
    fn serve_with_head_limit(
        runtime: &tokio::runtime::Runtime,
        (max_connections, idle_limit): (usize, Duration),
        head_limit: Duration,
    ) -> TestResult<(u16, watch::Sender<bool>)> {
        let listener = runtime.block_on(TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))?;
        let port = listener.local_addr()?.port();
        let (stop, stopped) = watch::channel(false);
        let capped = Capped::new(listener, max_connections, idle_limit, stopped);
        let router = Router::new().fallback(|| async { "ok" });
        runtime.spawn(serve_capped(capped, router, head_limit));
        Ok((port, stop))
    }

    /// A runtime of two threads with a clock that runs, serving while the test's thread waits.
    fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
    }

    /// A connection to `port` that sent `head`, a request without its blank line.
    fn sent(port: u16, head: &str) -> std::io::Result<std::net::TcpStream> {
        let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        stream.set_read_timeout(Some(WAIT))?;
        // A connection closed on arrival may refuse the write; the read says what happened.
        let _ = stream.write_all(format!("{head}\r\n\r\n").as_bytes());
        Ok(stream)
    }

    /// Whether `stream` got the answer `ok` to its request.
    fn is_answered(stream: &mut std::net::TcpStream) -> bool {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 512];
        while !bytes.ends_with(b"ok") {
            match stream.read(&mut buffer) {
                Ok(read @ 1..) => bytes.extend_from_slice(buffer.get(..read).unwrap_or_default()),
                _ => return false,
            }
        }
        true
    }

    /// Whether the server closed `stream`: it reads its end, or the connection was reset, rather
    /// than waiting [`WAIT`] for a byte.
    fn is_closed(stream: &mut std::net::TcpStream) -> bool {
        match stream.read(&mut [0u8; 512]) {
            Ok(read) => read == 0,
            Err(error) => !matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
        }
    }

    /// A connection to `port` whose request `head` was answered, tried until one is, within
    /// [`WAIT`]: a slot given back is free a moment after its connection closed.
    fn answered(port: u16, head: &str) -> TestResult<std::net::TcpStream> {
        let until = Instant::now() + WAIT;
        while Instant::now() < until {
            let mut stream = sent(port, head)?;
            if is_answered(&mut stream) {
                return Ok(stream);
            }
        }
        Err("no connection was answered".into())
    }

    /// The request every test sends.
    const GET: &str = "GET / HTTP/1.1\r\nHost: t";

    /// With as many connections open as the cap allows, each answered and kept open, one more is
    /// closed without an answer; once one of them closes, a new one is answered.
    #[test]
    fn the_cap_holds_and_a_closed_connection_frees_its_slot() -> TestResult {
        let runtime = runtime()?;
        let (port, _stop) = serve(&runtime, MAX_CONNECTIONS, IDLE_LIMIT)?;
        let mut open = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            let mut stream = sent(port, GET)?;
            assert!(is_answered(&mut stream));
            open.push(stream);
        }
        let mut one_more = sent(port, GET)?;
        assert!(is_closed(&mut one_more));
        drop(open.pop());
        answered(port, GET)?;
        Ok(())
    }

    /// With one slot and an idle limit of 300 ms: a connection that sends nothing is closed, not
    /// before the limit, and its slot is free again; so is one that announced a body of a
    /// gigabyte and sent none of it, once its answer is written.
    #[test]
    fn an_idle_connection_is_closed_and_frees_its_slot() -> TestResult {
        let runtime = runtime()?;
        let limit = Duration::from_millis(300);
        let (port, _stop) = serve(&runtime, 1, limit)?;
        let started = Instant::now();
        let mut silent = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        silent.set_read_timeout(Some(WAIT))?;
        assert!(is_closed(&mut silent));
        assert!(started.elapsed() >= limit);
        drop(answered(port, GET)?);
        let announced = "POST / HTTP/1.1\r\nHost: t\r\nContent-Length: 1000000000";
        let mut announced = answered(port, announced)?;
        assert!(is_closed(&mut announced));
        answered(port, GET)?;
        Ok(())
    }

    /// With one slot, an idle limit of 30 s and a head limit of 1 s: a client that sends its head
    /// one byte every 100 ms, never idle, never finishing it, is closed once the head limit has
    /// passed, long before the idle limit, and its slot is free again. Without the head limit only
    /// the idle limit would end it, and the dribble keeps it from ever being idle.
    #[test]
    fn a_head_dribbled_byte_by_byte_is_cut_at_the_head_limit() -> TestResult {
        let runtime = runtime()?;
        let head_limit = Duration::from_secs(1);
        let (port, _stop) = serve_with_head_limit(&runtime, (1, IDLE_LIMIT), head_limit)?;
        let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        let started = Instant::now();
        let mut is_cut = false;
        for byte in "GET / HTTP/1.1\r\nX-Slow: "
            .bytes()
            .chain(std::iter::repeat(b'a'))
        {
            if started.elapsed() > WAIT {
                break;
            }
            if stream.write_all(&[byte]).is_err() {
                is_cut = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let elapsed = started.elapsed();
        assert!(is_cut, "still open after {elapsed:?}");
        assert!(elapsed >= head_limit, "cut after {elapsed:?}");
        // The limit, a write or two to see the cut, and room for a loaded machine: well short of
        // the idle limit, which is what would end it without the head limit.
        assert!(elapsed < head_limit * 5, "cut after {elapsed:?}");
        answered(port, GET)?;
        Ok(())
    }

    /// With a head limit of 1 s and an idle limit of 30 s, a kept-alive connection: two more
    /// requests, each 800 ms after the answer before it, are answered on the same connection, the
    /// third 1.6 s after the connection opened, which a limit started once would have cut; left
    /// silent after that, the connection is closed by the head limit, before 5 s, long before the
    /// idle limit.
    #[test]
    fn the_head_limit_runs_again_for_each_request() -> TestResult {
        let runtime = runtime()?;
        let head_limit = Duration::from_secs(1);
        let (port, _stop) = serve_with_head_limit(&runtime, (1, IDLE_LIMIT), head_limit)?;
        let opened = Instant::now();
        let mut stream = sent(port, GET)?;
        assert!(is_answered(&mut stream));
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(800));
            stream.write_all(format!("{GET}\r\n\r\n").as_bytes())?;
            assert!(is_answered(&mut stream));
        }
        assert!(opened.elapsed() > head_limit, "{:?}", opened.elapsed());
        let silent = Instant::now();
        assert!(is_closed(&mut stream));
        let elapsed = silent.elapsed();
        assert!(
            elapsed >= head_limit && elapsed < head_limit * 5,
            "{elapsed:?}"
        );
        Ok(())
    }

    /// An answer that says `Connection: close`, as every refusal of the guard does, ends its
    /// connection once written: the client reads the answer, then the connection's end, with the
    /// limits far off.
    #[test]
    fn an_answer_saying_close_ends_its_connection() -> TestResult {
        let runtime = runtime()?;
        let listener = runtime.block_on(TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))?;
        let port = listener.local_addr()?.port();
        let (_stop, stopped) = watch::channel(false);
        let capped = Capped::new(listener, 1, IDLE_LIMIT, stopped);
        let router = Router::new()
            .fallback(|| async { ([(axum::http::header::CONNECTION, "close")], "ok") });
        runtime.spawn(serve_capped(capped, router, HEAD_LIMIT));
        let mut stream = sent(port, GET)?;
        let started = Instant::now();
        assert!(is_answered(&mut stream));
        assert!(is_closed(&mut stream));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        Ok(())
    }
}
