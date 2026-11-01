//! Asking a daemon older than this build to stop, so that one of this build can take its place,
//! or any daemon, for a command that takes Cortex out of a repository.
//!
//! The handshake is read first and the stop sent on the same connection, only when the handshake
//! shows the daemon older: a daemon that replaced the older one meanwhile is the one that answers,
//! and it is left running. A newer daemon is never stopped, so two installed versions never take
//! turns replacing each other.

use std::io::Write as _;
use std::time::Instant;

use cortex_core::RunFiles;
use cortex_core::ipc::{
    Request, RequestBody, Response, Role, is_older_than_this_build, read_frame,
};

use crate::client::{self, ClientError, REQUEST_ID};

/// What a request to stop an older daemon found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The daemon, of this Cortex version, was older than this build and was asked to stop.
    Asked(String),
    /// The daemon, of this Cortex version, was not older than this build and was left running.
    Declined(String),
}

/// Connects to the daemon whose run files are `files`, introduces this process as `role`, and asks
/// it to stop only when its handshake shows it older than this build, all before `deadline`. A
/// daemon of another protocol is told to stop all the same when older: every version reads a stop.
///
/// # Errors
///
/// As [`client::ask`]: no daemon, no answer in time, another process at the endpoint, and a broken
/// exchange.
pub fn stop_if_older(files: &RunFiles, role: Role, deadline: Instant) -> Result<Stop, ClientError> {
    stop_when(files, role, deadline, is_older_than_this_build)
}

/// Connects to the daemon whose run files are `files`, introduces this process as `role`, and asks
/// it to stop whatever its version, all before `deadline`, and returns the Cortex version it
/// named. A daemon of another protocol reads a stop all the same.
///
/// # Errors
///
/// As [`stop_if_older`].
pub fn stop_any(files: &RunFiles, role: Role, deadline: Instant) -> Result<String, ClientError> {
    // Every version is one to stop, so the daemon is always asked.
    match stop_when(files, role, deadline, |_| true)? {
        Stop::Asked(version) | Stop::Declined(version) => Ok(version),
    }
}

/// Asks the daemon whose run files are `files` to stop, as `role`, before `deadline`, when the
/// version its handshake gives is one `should_stop` takes, on the connection that read it.
fn stop_when(
    files: &RunFiles,
    role: Role,
    deadline: Instant,
    should_stop: fn(&str) -> bool,
) -> Result<Stop, ClientError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(ClientError::TimedOut)?;
    let hello = client::encoded(&client::hello(role))?;
    let stop = client::encoded(&Request {
        request_id: REQUEST_ID,
        body: RequestBody::Shutdown,
    })?;
    let endpoint = files.endpoint();
    let daemon_pid_path = files.pid_path();
    client::on_thread(remaining, move || {
        let mut stream = client::connect(&endpoint, &daemon_pid_path, remaining)?;
        stream
            .write_all(&hello)
            .and_then(|()| stream.flush())
            .map_err(ClientError::Io)?;
        let version = match client::read_handshake(&mut stream)? {
            Ok(welcome) => welcome.cortex_version,
            Err(mismatch) => mismatch.cortex_version,
        };
        if !should_stop(&version) {
            return Ok(Stop::Declined(version));
        }
        stream
            .write_all(&stop)
            .and_then(|()| stream.flush())
            .map_err(ClientError::Io)?;
        if read_frame::<Response>(&mut stream)?.request_id != REQUEST_ID {
            return Err(ClientError::Protocol("an answer names another request"));
        }
        Ok(Stop::Asked(version))
    })
}
