//! A connection that cannot outlive its use: it ends once no byte has gone either way for its idle
//! limit, and as soon as the map stops.
//!
//! The server bounds how long it waits for a request's head (the map's head limit), but nothing
//! else: without this a client that announces a body it never sends, or stops reading an answer,
//! would hold its connection, and with it one of the map's few slots, for as long as the daemon
//! runs. Every byte read or written moves the deadline on, so a connection in use, a web socket
//! whose pings go back and forth included, is never cut. Once the deadline passes, or the map stops, the next read, write
//! or flush the server waits on fails, and the server drops the connection.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::watch;
use tokio::time::{Instant, Sleep};

/// A future that ends when the map stops.
type Stopped = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A connection that ends when idle for its limit or when the map stops.
pub(crate) struct Bounded<S> {
    /// The connection.
    stream: S,
    /// How long it may go with no byte either way.
    idle_limit: Duration,
    /// When it ends unless a byte goes either way first.
    deadline: Pin<Box<Sleep>>,
    /// Ends when the map stops.
    stopped: Stopped,
}

impl<S> std::fmt::Debug for Bounded<S> {
    /// The limit alone: the rest is the connection's own state.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Bounded")
            .field("idle_limit", &self.idle_limit)
            .finish_non_exhaustive()
    }
}

impl<S> Bounded<S> {
    /// `stream`, ended when idle for `idle_limit` or once `stop` holds true. Called inside the
    /// daemon's runtime, whose clock the deadline runs on.
    pub(crate) fn new(stream: S, idle_limit: Duration, mut stop: watch::Receiver<bool>) -> Self {
        Self {
            stream,
            idle_limit,
            deadline: Box::pin(tokio::time::sleep(idle_limit)),
            stopped: Box::pin(async move {
                // A sender dropped without a stop means the map is gone: the connection ends too.
                let _ = stop.wait_for(|is_stopped| *is_stopped).await;
            }),
        }
    }

    /// Moves the deadline on: a byte went one way or the other.
    fn moved(&mut self) {
        let next = Instant::now() + self.idle_limit;
        self.deadline.as_mut().reset(next);
    }

    /// Why the connection ends, when it does now: the map stopped, or the deadline passed. Polling
    /// both registers the task to be woken when either happens.
    fn ended(&mut self, context: &mut Context<'_>) -> Option<io::Error> {
        if self.stopped.as_mut().poll(context).is_ready() {
            return Some(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "the map stopped",
            ));
        }
        if self.deadline.as_mut().poll(context).is_ready() {
            return Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "the connection was idle too long",
            ));
        }
        None
    }

    /// `polled`, the stream's answer, unless it is still waiting and the connection has ended.
    fn unless_ended<T>(
        &mut self,
        context: &mut Context<'_>,
        polled: Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        match polled {
            Poll::Pending => self
                .ended(context)
                .map_or(Poll::Pending, |why| Poll::Ready(Err(why))),
            Poll::Ready(answer) => Poll::Ready(answer),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Bounded<S> {
    /// Reads from the connection, moving the deadline on when bytes arrive.
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(why) = this.ended(context) {
            return Poll::Ready(Err(why));
        }
        let before = buffer.filled().len();
        let polled = Pin::new(&mut this.stream).poll_read(context, buffer);
        if matches!(polled, Poll::Ready(Ok(()))) && buffer.filled().len() > before {
            this.moved();
        }
        this.unless_ended(context, polled)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Bounded<S> {
    /// Writes to the connection, moving the deadline on when bytes leave.
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let Some(why) = this.ended(context) {
            return Poll::Ready(Err(why));
        }
        let polled = Pin::new(&mut this.stream).poll_write(context, bytes);
        if matches!(polled, Poll::Ready(Ok(written)) if written > 0) {
            this.moved();
        }
        this.unless_ended(context, polled)
    }

    /// Flushes the connection.
    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.stream).poll_flush(context);
        this.unless_ended(context, polled)
    }

    /// Shuts the connection's writing half.
    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.stream).poll_shutdown(context);
        this.unless_ended(context, polled)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    use super::*;

    /// The idle limit in these tests.
    const LIMIT: Duration = Duration::from_secs(30);

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Runs `test` on a runtime whose clock stands still and moves on to the next deadline
    /// whenever nothing else can run, so that waits of 30 s take no time and end exactly.
    fn on_paused_clock(test: impl AsyncFnOnce() -> TestResult) -> TestResult {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()?
            .block_on(test())
    }

    /// A pair: the client's end, and the server's end bounded at [`LIMIT`] with a stop never sent.
    fn pair() -> (DuplexStream, Bounded<DuplexStream>, watch::Sender<bool>) {
        let (client, server) = tokio::io::duplex(64);
        let (stop, stopped) = watch::channel(false);
        (client, Bounded::new(server, LIMIT, stopped), stop)
    }

    /// A read of at most `size` bytes from `server`, given up as a failed test if it has not ended
    /// three limits later, so that a connection that never ends fails rather than hangs.
    async fn bounded_read(
        server: &mut Bounded<DuplexStream>,
        size: usize,
    ) -> Result<io::Result<usize>, tokio::time::error::Elapsed> {
        tokio::time::timeout(LIMIT * 3, server.read(&mut vec![0u8; size])).await
    }

    /// A connection over which nothing comes ends exactly at the limit, with a timeout.
    #[test]
    fn a_silent_connection_ends_at_the_limit() -> TestResult {
        on_paused_clock(async || {
            let (_client, mut server, _stop) = pair();
            let started = Instant::now();
            let read = bounded_read(&mut server, 8).await?;
            assert_eq!(
                read.map_err(|error| error.kind()),
                Err(io::ErrorKind::TimedOut)
            );
            assert_eq!(started.elapsed(), LIMIT);
            Ok(())
        })
    }

    /// A byte each way moves the deadline on: a byte from the client after 20 s, then one from
    /// the server 20 s after that, so the next wait ends 30 s after the last byte, 70 s in.
    #[test]
    fn bytes_either_way_move_the_deadline_on() -> TestResult {
        on_paused_clock(async || {
            let (mut client, mut server, _stop) = pair();
            let started = Instant::now();
            tokio::time::advance(Duration::from_secs(20)).await;
            client.write_all(b"a").await?;
            assert_eq!(server.read(&mut [0u8; 1]).await?, 1);
            tokio::time::advance(Duration::from_secs(20)).await;
            server.write_all(b"b").await?;
            let read = bounded_read(&mut server, 1).await?;
            assert_eq!(
                read.map_err(|error| error.kind()),
                Err(io::ErrorKind::TimedOut)
            );
            assert_eq!(started.elapsed(), Duration::from_secs(70));
            Ok(())
        })
    }

    /// A connection waiting for bytes ends as soon as the map stops, long before its limit.
    #[test]
    fn a_stop_ends_a_waiting_connection_at_once() -> TestResult {
        on_paused_clock(async || {
            let (_client, mut server, stop) = pair();
            let started = Instant::now();
            let stopper = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                stop.send_replace(true);
                stop
            });
            let read = bounded_read(&mut server, 8).await?;
            assert_eq!(
                read.map_err(|error| error.kind()),
                Err(io::ErrorKind::ConnectionAborted)
            );
            assert_eq!(started.elapsed(), Duration::from_secs(1));
            drop(stopper.await?);
            Ok(())
        })
    }
}
