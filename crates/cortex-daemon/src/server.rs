//! The daemon's side of the protocol: accept connections, answer the handshake, then each request,
//! until the client closes, asks to stop, or breaks the protocol.
//!
//! Each connection is a task of the tokio runtime; at most [`MAX_CONNECTIONS`] are served at once
//! and a frame must arrive within [`FRAME_TIMEOUT`], so no client can tie the daemon up.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cortex_core::ipc::{
    CORTEX_VERSION, FrameError, LENGTH_PREFIX_BYTES, Mismatch, Protocol, Request, RequestBody,
    Response, ResponseBody, StatusReport, Welcome, body_len, decode, encode,
};
use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::Listener as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;

use crate::inbox::Inbox;

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
    /// The writer's inbox, which a Shutdown stops.
    inbox: Arc<Inbox>,
}

impl Server {
    /// A server for a daemon started now, whose writer reads `inbox`.
    pub(crate) fn new(inbox: Arc<Inbox>) -> Self {
        Self {
            started: Instant::now(),
            inbox,
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
                // A connection that breaks ends; the daemon goes on.
                drop(server.connection(stream).await);
                drop(permit);
            });
        }
    }

    /// Serves one connection: the handshake, then each request in turn.
    async fn connection(&self, stream: Stream) -> Result<(), FrameError> {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let hello = read_request(&mut reader).await?;
        let RequestBody::Hello(greeting) = hello.body else {
            return Ok(());
        };
        if !Protocol::CURRENT.is_compatible_with(greeting.protocol) {
            let mismatch = ResponseBody::Mismatch(Mismatch {
                protocol: Protocol::CURRENT,
                cortex_version: CORTEX_VERSION.to_owned(),
            });
            write_response(&mut writer, hello.request_id, mismatch).await?;
            // A client of another protocol may still send the one request every version reads.
            if let Ok(next) = read_request(&mut reader).await
                && next.body == RequestBody::Shutdown
            {
                write_response(&mut writer, next.request_id, ResponseBody::Ack).await?;
                self.inbox.stop();
            }
            return Ok(());
        }
        let welcome = ResponseBody::Welcome(Welcome {
            protocol: Protocol::CURRENT,
            cortex_version: CORTEX_VERSION.to_owned(),
            pid: std::process::id(),
        });
        write_response(&mut writer, hello.request_id, welcome).await?;
        loop {
            let Ok(request) = read_request(&mut reader).await else {
                return Ok(());
            };
            let is_shutdown = matches!(request.body, RequestBody::Shutdown);
            let (body, is_last) = match request.body {
                RequestBody::Status => (ResponseBody::Status(self.report()), false),
                RequestBody::Shutdown => (ResponseBody::Ack, true),
                RequestBody::Hello(_) => (error("a connection says hello once"), true),
                RequestBody::Event { .. } => (error("this daemon does not take events yet"), false),
            };
            write_response(&mut writer, request.request_id, body).await?;
            if is_shutdown {
                self.inbox.stop();
            }
            if is_last {
                return Ok(());
            }
        }
    }

    /// The daemon's state.
    fn report(&self) -> StatusReport {
        StatusReport {
            cortex_version: CORTEX_VERSION.to_owned(),
            protocol: Protocol::CURRENT,
            pid: std::process::id(),
            uptime_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }
}

/// An answer saying a request could not be served.
fn error(message: &str) -> ResponseBody {
    ResponseBody::Error {
        message: message.to_owned(),
    }
}

/// Reads one request, which must arrive within [`FRAME_TIMEOUT`].
async fn read_request(reader: &mut (impl AsyncRead + Unpin)) -> Result<Request, FrameError> {
    let read = async {
        let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
        reader.read_exact(&mut prefix).await?;
        let mut body = vec![0u8; body_len(prefix)?];
        reader.read_exact(&mut body).await?;
        decode(&body)
    };
    tokio::time::timeout(FRAME_TIMEOUT, read)
        .await
        .map_err(|_| FrameError::Io(io::ErrorKind::TimedOut.into()))?
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
