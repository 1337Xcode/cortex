//! The daemon's side of the protocol: accept connections, answer the handshake, then each request,
//! until the client closes, asks to stop, or breaks the protocol.
//!
//! Each connection is a task of the tokio runtime; at most [`MAX_CONNECTIONS`] are served at once
//! and a frame must arrive within [`FRAME_TIMEOUT`], so no client can tie the daemon up. A
//! connection that breaks is kept among the daemon's recent errors, in the daemon's own words and
//! never with the client's bytes, and the time each hook's request took to handle is kept for the
//! daemon's status.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cortex_core::HookKind;
use cortex_core::ipc::{
    CORTEX_VERSION, FrameError, LENGTH_PREFIX_BYTES, Mismatch, Protocol, Request, RequestBody,
    Response, ResponseBody, StatusReport, Welcome, body_len, decode, encode,
};
use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::Listener as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;

use crate::host::now_ms;
use crate::inbox::Inbox;
use crate::latency::Latencies;

/// Most connections served at once. Hooks and commands hold a connection for one exchange, so more
/// at once means something is flooding the daemon; a connection past this is closed at once.
const MAX_CONNECTIONS: usize = 64;

/// How long a frame may take to arrive once its connection opened or the previous frame was read.
/// Hooks and commands send at once; a client that does not is closed.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// What the connections share.
#[derive(Debug)]
pub(crate) struct Server {
    /// When the daemon started.
    started: Instant,
    /// The writer's inbox, which a Shutdown stops and which keeps the daemon's recent errors.
    inbox: Arc<Inbox>,
    /// How long each hook's latest requests took to handle.
    latencies: Mutex<Latencies>,
}

/// A request as read, with when the first byte of its frame arrived.
#[derive(Debug)]
struct Received {
    /// The request.
    request: Request,
    /// When its frame began to arrive.
    began: Instant,
}

/// Why a connection broke off.
#[derive(Debug)]
enum Broken {
    /// A request could not be read.
    Unreadable(FrameError),
    /// The first request was not the handshake.
    NoHello,
    /// The handshake came a second time.
    HelloAgain,
    /// An answer could not be written.
    Unwritable(FrameError),
}

impl Broken {
    /// What broke, for the daemon's recent errors, in words of the daemon's own.
    fn message(&self) -> String {
        match self {
            Self::Unreadable(FrameError::Io(error)) if error.kind() == io::ErrorKind::TimedOut => {
                format!(
                    "a request was cut off: the rest of its frame did not arrive within {} s",
                    FRAME_TIMEOUT.as_secs()
                )
            }
            Self::Unreadable(FrameError::Io(_)) => {
                "a request was cut off: the connection closed in the middle of its frame".to_owned()
            }
            // INVARIANT: no-content-stored (a frame's own error names its kind or size only; the
            // JSON error inside it, which can quote the client's text, is left out)
            Self::Unreadable(error) => format!("a request could not be read: {error}"),
            Self::NoHello => "a client asked before its handshake".to_owned(),
            Self::HelloAgain => "a client sent its handshake twice".to_owned(),
            // The operating system's reason, such as a pipe the client had already closed.
            Self::Unwritable(FrameError::Io(error)) => {
                format!("an answer could not be written: {error}")
            }
            Self::Unwritable(error) => format!("an answer could not be written: {error}"),
        }
    }
}

impl Server {
    /// A server for a daemon started now, whose writer reads `inbox`.
    pub(crate) fn new(inbox: Arc<Inbox>) -> Self {
        Self {
            started: Instant::now(),
            inbox,
            latencies: Mutex::new(Latencies::default()),
        }
    }

    /// Accepts connections on `listener` and serves each on a task of its own, until the runtime
    /// shuts down.
    pub(crate) async fn serve(self: Arc<Self>, listener: Listener) {
        let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            // A connection that fails while being accepted concerns that client alone.
            let Ok(stream) = listener.accept().await else {
                continue;
            };
            let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                drop(stream);
                continue;
            };
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                server.connection(stream).await;
                drop(permit);
            });
        }
    }

    /// Serves one connection. One that breaks ends, and what broke it is kept among the recent
    /// errors before the connection closes, so that a client that sees it closed finds the error
    /// in the status; the daemon goes on.
    async fn connection(&self, stream: Stream) {
        let (mut reader, mut writer) = tokio::io::split(stream);
        if let Err(broken) = self.exchange(&mut reader, &mut writer).await {
            self.note(&broken);
        }
    }

    /// The handshake, then each request in turn, until the client closes or asks to stop.
    async fn exchange(
        &self,
        reader: &mut (impl AsyncRead + Unpin),
        writer: &mut (impl AsyncWrite + Unpin),
    ) -> Result<(), Broken> {
        let Some(Received { request: hello, .. }) =
            read_request(reader).await.map_err(Broken::Unreadable)?
        else {
            return Ok(());
        };
        self.inbox.note_request();
        let RequestBody::Hello(greeting) = hello.body else {
            return Err(Broken::NoHello);
        };
        if !Protocol::CURRENT.is_compatible_with(greeting.protocol) {
            return self.mismatch(hello.request_id, reader, writer).await;
        }
        let welcome = ResponseBody::Welcome(Welcome {
            protocol: Protocol::CURRENT,
            cortex_version: CORTEX_VERSION.to_owned(),
            pid: std::process::id(),
        });
        write_response(writer, hello.request_id, welcome)
            .await
            .map_err(Broken::Unwritable)?;
        while let Some(received) = read_request(reader).await.map_err(Broken::Unreadable)? {
            self.inbox.note_request();
            if !self.answer(received, writer).await? {
                break;
            }
        }
        Ok(())
    }

    /// Answers the handshake of a client of another protocol, `request_id`, with this daemon's
    /// protocol and version. The client may still send the one request every version reads, a
    /// stop; anything else, which this daemon may not be able to read, ends the connection.
    async fn mismatch(
        &self,
        request_id: u64,
        reader: &mut (impl AsyncRead + Unpin),
        writer: &mut (impl AsyncWrite + Unpin),
    ) -> Result<(), Broken> {
        let mismatch = ResponseBody::Mismatch(Mismatch {
            protocol: Protocol::CURRENT,
            cortex_version: CORTEX_VERSION.to_owned(),
        });
        write_response(writer, request_id, mismatch)
            .await
            .map_err(Broken::Unwritable)?;
        if let Ok(Some(Received { request: next, .. })) = read_request(reader).await
            && next.body == RequestBody::Shutdown
        {
            let acknowledged = write_response(writer, next.request_id, ResponseBody::Ack).await;
            // A stop holds whether or not the client stays for its acknowledgement.
            self.inbox.stop();
            acknowledged.map_err(Broken::Unwritable)?;
        }
        Ok(())
    }

    /// Answers the request `received`, and returns whether the connection goes on: not after a
    /// stop or a second handshake. The time a hook's request took, from the first byte of its
    /// frame until its answer is ready to write, is kept.
    async fn answer(
        &self,
        received: Received,
        writer: &mut (impl AsyncWrite + Unpin),
    ) -> Result<bool, Broken> {
        let Received { request, began } = received;
        let is_shutdown = request.body == RequestBody::Shutdown;
        let (body, hook) = match request.body {
            RequestBody::Status => (ResponseBody::Status(self.report()), None),
            RequestBody::Shutdown => (ResponseBody::Ack, None),
            RequestBody::Hello(_) => {
                // Kept before the refusal is written, so that a client that reads it finds the
                // error in the status, as one that sees a broken connection close does.
                self.note(&Broken::HelloAgain);
                let refusal = error("a connection says hello once");
                write_response(writer, request.request_id, refusal)
                    .await
                    .map_err(Broken::Unwritable)?;
                return Ok(false);
            }
            RequestBody::Event { event } => {
                let hook = event.hook;
                // The moment before a tool runs is answered, not recorded: the tool's result is,
                // once it ran.
                if hook != HookKind::PreTool {
                    self.inbox.push_event(*event);
                }
                (ResponseBody::Ack, Some(hook))
            }
        };
        let took = began.elapsed();
        if let Some(hook) = hook {
            self.latencies().note(hook, took);
        }
        let answered = write_response(writer, request.request_id, body).await;
        if is_shutdown {
            // A stop holds whether or not the client stays for its acknowledgement.
            self.inbox.stop();
        }
        answered.map_err(Broken::Unwritable)?;
        // Logged once the hook has its answer: the line costs it nothing.
        if let Some(hook) = hook {
            let latency_us = u64::try_from(took.as_micros()).unwrap_or(u64::MAX);
            tracing::info!(
                request_id = request.request_id,
                hook = hook.name(),
                latency_us,
                "hook handled"
            );
        }
        Ok(!is_shutdown)
    }

    /// Keeps what broke a connection among the recent errors, and logs it.
    fn note(&self, broken: &Broken) {
        let message = broken.message();
        tracing::warn!(error = %message, "a connection broke");
        self.inbox.note_error(&message, now_ms());
    }

    /// The daemon's state.
    fn report(&self) -> StatusReport {
        // The times are copied out, so that connections noting theirs wait for the copy only, not
        // for the sorting, and the inbox is never locked while the times are.
        let latencies = self.latencies().clone();
        let counts = self.inbox.counts();
        StatusReport {
            cortex_version: CORTEX_VERSION.to_owned(),
            protocol: Protocol::CURRENT,
            pid: std::process::id(),
            uptime_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            events_waiting: u64::try_from(counts.events_waiting).unwrap_or(u64::MAX),
            changes_waiting: u64::try_from(counts.changes_waiting).unwrap_or(u64::MAX),
            events_dropped: counts.events_dropped,
            failed_batches: counts.failed_batches,
            failed_checkpoints: counts.failed_checkpoints,
            latencies: latencies.report(),
            recent_errors: self.inbox.recent_errors(),
        }
    }

    /// The hooks' handling times, locked. A panic while they were held leaves them consistent
    /// (each time is noted in one push), so a poisoned lock is taken as it is.
    fn latencies(&self) -> MutexGuard<'_, Latencies> {
        self.latencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// An answer saying a request could not be served.
fn error(message: &str) -> ResponseBody {
    ResponseBody::Error {
        message: message.to_owned(),
    }
}

/// Reads one request, whose whole frame must arrive within [`FRAME_TIMEOUT`]. A client that closes
/// the connection, says nothing for that long, or whose connection fails before a frame begins is
/// done, and that is no error: `None`.
///
/// # Errors
///
/// A frame cut off or late, one too long, and one that is not a request.
async fn read_request(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<Option<Received>, FrameError> {
    let deadline = tokio::time::Instant::now() + FRAME_TIMEOUT;
    let Ok(Ok(first)) = tokio::time::timeout_at(deadline, reader.read_u8()).await else {
        return Ok(None);
    };
    let began = Instant::now();
    let rest = async {
        let mut others = [0u8; LENGTH_PREFIX_BYTES - 1];
        reader.read_exact(&mut others).await?;
        let [second, third, fourth] = others;
        let mut body = vec![0u8; body_len([first, second, third, fourth])?];
        reader.read_exact(&mut body).await?;
        decode(&body)
    };
    let request = tokio::time::timeout_at(deadline, rest)
        .await
        .map_err(|_| FrameError::Io(io::ErrorKind::TimedOut.into()))??;
    Ok(Some(Received { request, began }))
}

/// Writes the response to request `request_id`.
async fn write_response(
    writer: &mut (impl AsyncWrite + Unpin),
    request_id: u64,
    body: ResponseBody,
) -> Result<(), FrameError> {
    writer
        .write_all(&encode(&Response { request_id, body })?)
        .await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::io::DuplexStream;

    use super::*;
    use crate::inbox::Work;

    /// Result type for these tests: any failure fails the test.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Runs `read_request` on the reading end of a pipe after `send` has used the other end, on a
    /// runtime whose clock moves on whenever nothing else can, so that a wait of 10 s takes none.
    fn read_after(
        send: impl AsyncFnOnce(&mut DuplexStream) -> io::Result<()>,
    ) -> Result<Result<Option<Received>, FrameError>, Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()?;
        runtime.block_on(async {
            let (mut client, mut daemon) = tokio::io::duplex(64);
            send(&mut client).await?;
            let read = read_request(&mut daemon).await;
            drop(client);
            Ok(read)
        })
    }

    /// A client that closes, or that says nothing for 10 s, before a frame begins is done: no
    /// request and no error. A whole frame is a request.
    #[test]
    fn a_frame_not_begun_is_no_error() -> TestResult {
        let closed = read_after(async |client| client.shutdown().await)?;
        assert!(matches!(closed, Ok(None)), "{closed:?}");
        let silent = read_after(async |_| Ok(()))?;
        assert!(matches!(silent, Ok(None)), "{silent:?}");
        let status = encode(&Request {
            request_id: 3,
            body: RequestBody::Status,
        })?;
        let whole = read_after(async |client| client.write_all(&status).await)?;
        let request = whole?.map(|received| received.request);
        assert_eq!(
            request,
            Some(Request {
                request_id: 3,
                body: RequestBody::Status
            })
        );
        Ok(())
    }

    /// Two bytes of a prefix and then a close is a frame cut off; two bytes and then nothing for
    /// 10 s is a late one. Each is kept in words of its own.
    #[test]
    fn a_frame_begun_and_not_finished_is_an_error() -> TestResult {
        let cut = read_after(async |client| {
            client.write_all(&[5, 0]).await?;
            client.shutdown().await
        })?;
        let late = read_after(async |client| client.write_all(&[5, 0]).await)?;
        let messages: Vec<String> = [cut, late]
            .into_iter()
            .map(|read| match read {
                Err(error) => Broken::Unreadable(error).message(),
                Ok(_) => "read".to_owned(),
            })
            .collect();
        assert_eq!(
            messages,
            [
                "a request was cut off: the connection closed in the middle of its frame",
                "a request was cut off: the rest of its frame did not arrive within 10 s",
            ]
        );
        Ok(())
    }

    /// A stop holds though its acknowledgement cannot be written, the client having gone: the
    /// answer is a broken pipe, and the writer is told to stop all the same.
    #[test]
    fn a_stop_holds_when_its_answer_cannot_be_written() -> TestResult {
        let inbox = Arc::new(Inbox::default());
        let server = Server::new(Arc::clone(&inbox));
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        let answered = runtime.block_on(async {
            let (client, mut daemon) = tokio::io::duplex(64);
            drop(client);
            let stop = Received {
                request: Request {
                    request_id: 2,
                    body: RequestBody::Shutdown,
                },
                began: Instant::now(),
            };
            server.answer(stop, &mut daemon).await
        });
        assert!(
            matches!(answered, Err(Broken::Unwritable(FrameError::Io(_)))),
            "{answered:?}"
        );
        assert_eq!(inbox.wait(Some(Instant::now())), Work::Stop);
        Ok(())
    }

    /// A frame that is not a request is kept as such, never with the JSON error, which quotes the
    /// client's text: here the text a request id cannot be.
    #[test]
    fn an_unreadable_request_keeps_none_of_the_client_s_text() -> TestResult {
        let Err(error) = decode::<Request>(br#"{"request_id":"private words","body":{}}"#) else {
            return Err("the frame was read".into());
        };
        assert_eq!(
            Broken::Unreadable(error).message(),
            "a request could not be read: a frame body is not the message expected"
        );
        let too_long = Broken::Unreadable(FrameError::TooLarge {
            len_bytes: 2_000_000,
        });
        assert_eq!(
            too_long.message(),
            "a request could not be read: a frame body of 2000000 bytes is longer than 1048576"
        );
        let gone = Broken::Unwritable(FrameError::Io(io::ErrorKind::BrokenPipe.into()));
        assert_eq!(
            gone.message(),
            "an answer could not be written: broken pipe"
        );
        Ok(())
    }
}
