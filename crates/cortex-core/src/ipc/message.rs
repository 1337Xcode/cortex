//! The requests a hook or the CLI sends a daemon, and the daemon's responses.
//!
//! Every frame is an envelope holding a `request_id` and a `body` tagged by its `type`. The first
//! request on a connection is a Hello. The envelope, Hello, Mismatch and Shutdown never change
//! shape, so a process of any version can tell a daemon of any other version to stop.

use serde::{Deserialize, Serialize};

use super::version::Protocol;
use crate::Event;
use crate::named::named_kind;

/// A request to a daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The id the response repeats; a client numbers its requests on one connection from 1.
    pub request_id: u64,
    /// What is asked.
    pub body: RequestBody,
}

/// What a request asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequestBody {
    /// The handshake, first on every connection. Frozen.
    Hello(Hello),
    /// Record a hook event.
    Event {
        /// The event.
        event: Box<Event>,
    },
    /// Report the daemon's state.
    Status,
    /// Stop gracefully. Frozen.
    Shutdown,
}

/// The handshake a client opens with. Frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// The protocol the client speaks.
    pub protocol: Protocol,
    /// The client's Cortex version.
    pub cortex_version: String,
    /// Who the client is; a role this version does not know reads as [`Role::Other`], so that a
    /// client of any version is still answered.
    #[serde(deserialize_with = "role_or_other")]
    pub role: Role,
}

named_kind! {
    /// Who opened a connection.
    Role {
        /// The hook client an agent runs.
        Hook => "hook",
        /// A `cortex` command a person runs.
        Cli => "cli",
        /// A client of a kind this version does not know.
        Other => "other",
    }
}

/// A role from its name, or [`Role::Other`] for a name this version does not know.
fn role_or_other<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Role, D::Error> {
    let name = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
    Ok(Role::from_name(&name).unwrap_or(Role::Other))
}

/// A daemon's response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    /// The id of the request this answers.
    pub request_id: u64,
    /// The answer.
    pub body: ResponseBody,
}

/// What a response says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseBody {
    /// The handshake succeeded.
    Welcome(Welcome),
    /// The client's protocol is not one the daemon speaks; the daemon closes the connection after
    /// a Shutdown or at once. Frozen.
    Mismatch(Mismatch),
    /// The request was taken.
    Ack,
    /// The daemon's state.
    Status(StatusReport),
    /// The request could not be served.
    Error {
        /// What went wrong, without file contents or prompt text.
        message: String,
    },
}

/// The daemon's side of a successful handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    /// The protocol the daemon speaks.
    pub protocol: Protocol,
    /// The daemon's Cortex version.
    pub cortex_version: String,
    /// The daemon's process id.
    pub pid: u32,
}

/// The daemon's side of a failed handshake. Frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mismatch {
    /// The protocol the daemon speaks.
    pub protocol: Protocol,
    /// The daemon's Cortex version.
    pub cortex_version: String,
}

/// What a daemon reports about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReport {
    /// The daemon's Cortex version.
    pub cortex_version: String,
    /// The protocol it speaks.
    pub protocol: Protocol,
    /// Its process id.
    pub pid: u32,
    /// How long it has run, in milliseconds.
    pub uptime_ms: u64,
}
