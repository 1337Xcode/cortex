//! The client side of the daemon's protocol: connect, say hello and ask in one write, and read the
//! two answers, all before a deadline. Everything blocks; nothing here starts an async runtime.
//!
//! A named pipe on Windows takes no read or write timeout, so the exchange runs on a thread of its
//! own and the caller waits for it only until the deadline. A thread still blocked then is left
//! behind: the hook process it belongs to exits right after, and the CLI soon after.

use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cortex_core::ipc::{
    CORTEX_VERSION, FrameError, Hello, Mismatch, Protocol, Request, RequestBody, Response,
    ResponseBody, Role, Welcome, encode, read_frame,
};
use cortex_core::{Endpoint, RunFiles};
use interprocess::ConnectWaitMode;
use interprocess::local_socket::traits::StreamCommon as _;
use interprocess::local_socket::{
    ConnectOptions, GenericFilePath, GenericNamespaced, Name, Stream, ToFsName, ToNsName,
};

/// The request id of the handshake, the first request on a connection.
const HELLO_ID: u64 = 1;

/// The request id of the request sent after the handshake.
const REQUEST_ID: u64 = 2;

/// Why a request got no answer.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// No daemon listens for the repository: nothing at its endpoint, or a socket nobody accepts
    /// on.
    #[error("no daemon is running for this repository")]
    NotRunning,
    /// No answer came before the deadline.
    #[error("the daemon did not answer in time")]
    TimedOut,
    /// The daemon speaks a protocol this client cannot.
    #[error(
        "the daemon (Cortex {}) speaks protocol {}.{}, this client {}.{}",
        .0.cortex_version, .0.protocol.major, .0.protocol.minor,
        Protocol::CURRENT.major, Protocol::CURRENT.minor
    )]
    Mismatch(Mismatch),
    /// An answer broke the protocol.
    #[error("the daemon's answer broke the protocol: {0}")]
    Protocol(&'static str),
    /// The request is longer than a frame may be; it was not sent.
    #[error("the request is longer than a frame may be")]
    TooLarge,
    /// The endpoint is held by a process other than the one the run files name as the daemon; it
    /// was told nothing.
    #[error("the endpoint is held by a process that is not this repository's daemon")]
    NotTheDaemon,
    /// Talking to the daemon failed.
    #[error("talking to the daemon failed")]
    Io(#[source] io::Error),
}

impl From<FrameError> for ClientError {
    /// A connection that failed is an I/O error; a frame that does not hold the answer expected
    /// breaks the protocol.
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Io(error) => Self::Io(error),
            FrameError::TooLarge { .. } => Self::Protocol("a frame is longer than allowed"),
            FrameError::Json(_) => Self::Protocol("a frame does not hold the answer expected"),
        }
    }
}

/// What the daemon answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The daemon's side of the handshake.
    pub welcome: Welcome,
    /// Its answer to the request.
    pub body: ResponseBody,
}

/// Connects to the daemon whose run files are `files`, introduces this process as `role`, sends
/// `body` and returns the daemon's answer, all before `deadline`.
///
/// # Errors
///
/// [`ClientError::NotRunning`] when no daemon listens, [`ClientError::TimedOut`] when the deadline
/// passes first (or has passed), [`ClientError::Mismatch`] when the daemon speaks another protocol,
/// [`ClientError::TooLarge`] for a request too long to send, [`ClientError::NotTheDaemon`] when
/// another process holds the endpoint, and the protocol and I/O errors of a broken exchange.
pub fn ask(
    files: &RunFiles,
    role: Role,
    body: RequestBody,
    deadline: Instant,
) -> Result<Answer, ClientError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(ClientError::TimedOut)?;
    let frames = frames_of(role, body)?;
    let endpoint = files.endpoint();
    let daemon_pid_path = files.pid_path();
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("cortex-ipc".to_owned())
        .spawn(move || {
            let exchanged = exchange(&endpoint, &daemon_pid_path, &frames, remaining);
            drop(sender.send(exchanged));
        })
        .map_err(ClientError::Io)?;
    // INVARIANT: fail-open (the caller waits for the daemon no longer than its deadline)
    receiver
        .recv_timeout(remaining)
        .unwrap_or(Err(ClientError::TimedOut))
}

/// The handshake as `role` and the request with `body`, framed one after the other. Encoded before
/// connecting, so that a request too long to send never reaches the endpoint.
fn frames_of(role: Role, body: RequestBody) -> Result<Vec<u8>, ClientError> {
    let hello = Request {
        request_id: HELLO_ID,
        body: RequestBody::Hello(Hello {
            protocol: Protocol::CURRENT,
            cortex_version: CORTEX_VERSION.to_owned(),
            role,
        }),
    };
    let request = Request {
        request_id: REQUEST_ID,
        body,
    };
    let encoded = |request: &Request| match encode(request) {
        Err(FrameError::TooLarge { .. }) => Err(ClientError::TooLarge),
        other => other.map_err(ClientError::from),
    };
    let mut frames = encoded(&hello)?;
    frames.extend(encoded(&request)?);
    Ok(frames)
}

/// Connects within `connect_within`, makes sure the daemon holds the endpoint, sends the frames in
/// one write, and reads both answers.
fn exchange(
    endpoint: &Endpoint,
    daemon_pid_path: &Path,
    frames: &[u8],
    connect_within: Duration,
) -> Result<Answer, ClientError> {
    let mut stream = ConnectOptions::new()
        .name(name_of(endpoint).map_err(ClientError::Io)?)
        .wait_mode(ConnectWaitMode::Timeout(connect_within))
        .connect_sync()
        .map_err(connect_error)?;
    check_peer(&stream, daemon_pid_path)?;
    stream
        .write_all(frames)
        .and_then(|()| stream.flush())
        .map_err(ClientError::Io)?;
    let welcome = match read_frame::<Response>(&mut stream)? {
        Response {
            request_id: HELLO_ID,
            body: ResponseBody::Welcome(welcome),
        } => welcome,
        Response {
            request_id: HELLO_ID,
            body: ResponseBody::Mismatch(mismatch),
        } => return Err(ClientError::Mismatch(mismatch)),
        _ => {
            return Err(ClientError::Protocol(
                "the first answer is not the handshake's",
            ));
        }
    };
    let answer = read_frame::<Response>(&mut stream)?;
    if answer.request_id != REQUEST_ID {
        return Err(ClientError::Protocol("an answer names another request"));
    }
    Ok(Answer {
        welcome,
        body: answer.body,
    })
}

/// Refuses an endpoint held by a process other than the daemon. On Windows any local user can
/// create a pipe of the daemon's name while no daemon runs, and would then be sent prompts: the
/// pipe server's process must be the one the user's own run folder names as the daemon. On Unix
/// the socket lives in a folder only the user can open, so whoever listens there is the user's.
fn check_peer(stream: &Stream, daemon_pid_path: &Path) -> Result<(), ClientError> {
    if cfg!(unix) {
        return Ok(());
    }
    let server = stream.peer_creds().map_err(ClientError::Io)?.pid();
    let daemon = std::fs::read_to_string(daemon_pid_path)
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok());
    if server.is_some() && server == daemon {
        Ok(())
    } else {
        Err(ClientError::NotTheDaemon)
    }
}

/// The socket name `endpoint` stands for: a socket file on Unix, a named pipe on Windows.
fn name_of(endpoint: &Endpoint) -> io::Result<Name<'_>> {
    match endpoint {
        Endpoint::Socket(path) => path.as_path().to_fs_name::<GenericFilePath>(),
        Endpoint::Pipe(pipe) => pipe.as_str().to_ns_name::<GenericNamespaced>(),
    }
}

/// What a failed connection means: nobody listens, nobody answered in time, or something else.
fn connect_error(error: io::Error) -> ClientError {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => ClientError::NotRunning,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ClientError::TimedOut,
        _ => ClientError::Io(error),
    }
}
