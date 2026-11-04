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
    /// Record a prompt's event, and answer with the work order the prompt gets, if any.
    Retrieve {
        /// The prompt's event.
        event: Box<Event>,
        /// When the hook stops reading the answer, in UTC milliseconds by the wall clock, the one
        /// clock two processes share (a step of it in between misjudges the time): a work order
        /// that cannot reach the hook by then gives the agent nothing, so it is sent without its
        /// text and recorded as not delivered. None from a client that waits as long as it takes
        /// (protocol 2.1 added it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer_by_ms: Option<i64>,
    },
    /// Record a session's start, and answer with the primer the session gets, if any.
    Primer {
        /// The session's start.
        event: Box<Event>,
        /// When the hook stops reading the answer, in UTC milliseconds by the wall clock, as for
        /// `Retrieve`: a primer that cannot reach the hook by then is sent without its text and
        /// recorded as not delivered. None from a client that waits as long as it takes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer_by_ms: Option<i64>,
    },
    /// Answer a question with what the engine finds for it without the gate, as `cortex where`
    /// asks: the files, their code and their reasons, and what the gate would have said.
    Where {
        /// The question, in the words a prompt would use.
        query: String,
        /// The most files to name, from 1 to 5.
        limit: u8,
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
    /// The text a request asked for; none when there is nothing to give, as when the gate stays
    /// shut.
    Reply {
        /// The text.
        text: Option<String>,
    },
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
    /// Hook events waiting for the writer.
    #[serde(default)]
    pub events_waiting: u64,
    /// File changes waiting for the writer.
    #[serde(default)]
    pub changes_waiting: u64,
    /// Hook events dropped because too many waited, since the daemon started.
    #[serde(default)]
    pub events_dropped: u64,
    /// Batches of events the writer could not record, since the daemon started.
    #[serde(default)]
    pub failed_batches: u64,
    /// Checkpoints of the store's write-ahead log that failed, since the daemon started.
    #[serde(default)]
    pub failed_checkpoints: u64,
    /// Probes, as `cortex doctor` sends, that the daemon answered since it started. It records
    /// none of them.
    #[serde(default)]
    pub probes_answered: u64,
    /// Whether this daemon has finished its first index of the repository (in huge mode, the
    /// inventory): what an earlier daemon left in the store says nothing of this one.
    #[serde(default)]
    pub is_indexed: bool,
    /// For each hook that sent a request since the daemon started, in hook order, how long the
    /// daemon took to handle its latest requests.
    #[serde(default)]
    pub latencies: Vec<HookLatency>,
    /// The daemon's latest errors, oldest first.
    #[serde(default)]
    pub recent_errors: Vec<RecentError>,
    /// What the daemon's co-change miner is doing; none from a daemon that does not say, as one
    /// older than the miner.
    #[serde(default)]
    pub git: Option<GitReport>,
    /// The graph retrieval spreads over, as last built, and how its rebuilds went; none before the
    /// first rebuild ended, or from a daemon that does not say.
    #[serde(default)]
    pub graph: Option<GraphReport>,
}

/// What a daemon's graph holds, as last built, and how its rebuilds went, for its status. With no
/// graph built yet, its size is 0.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GraphReport {
    /// Its nodes: files, symbols and folders.
    pub nodes: u64,
    /// Its links, each way of a link that runs both ways counted.
    pub links: u64,
    /// Whether it holds the co-change links of a complete mining.
    pub has_cochange: bool,
    /// How many graphs the daemon built since it started.
    pub builds: u64,
    /// How long the latest build took, in milliseconds.
    pub build_ms: u64,
    /// How many rebuilds failed since the daemon started; each was tried again.
    #[serde(default)]
    pub failed_builds: u64,
}

/// What a daemon's co-change miner is doing, for its status. A daemon's miner starts out
/// [`GitState::Waiting`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GitReport {
    /// Where the miner is.
    #[serde(deserialize_with = "git_state_or_other")]
    pub state: GitState,
    /// More about it in words, when there is more: which program read the history when `gix`
    /// could not, or why the last mining failed.
    pub detail: Option<String>,
}

named_kind! {
    /// Where a daemon's co-change miner is.
    GitState {
        /// Waiting for the daemon's first index, before which no file counts.
        Waiting => "waiting",
        /// The repository's root folder is not the top of a git repository.
        NoRepository => "no_repository",
        /// A mining is due: waiting for the index to take the files HEAD's move wrote, reading
        /// the history, or waiting for what it found to be written.
        Mining => "mining",
        /// What the store holds was mined from HEAD as it is.
        Ready => "ready",
        /// The last try failed; the next comes after a pause, or sooner when HEAD moves.
        Failed => "failed",
        /// A state this version does not know.
        Other => "other",
    }
}

impl Default for GitState {
    /// A daemon's miner waits for the first index before anything else.
    fn default() -> Self {
        Self::Waiting
    }
}

/// A miner's state from its name, or [`GitState::Other`] for a name this version does not know,
/// so that a newer daemon's status still reads.
fn git_state_or_other<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<GitState, D::Error> {
    let name = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
    Ok(GitState::from_name(&name).unwrap_or(GitState::Other))
}

/// How long a daemon took to handle one hook's latest requests, each from the first byte of the
/// request to its answer being ready to write, as percentiles in microseconds. The hook's own
/// process, its connection and the handshake are not in these times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookLatency {
    /// The hook's name, as `HookKind::name` gives it: a name rather than a kind, so that a report
    /// from a newer daemon, naming a hook this build does not know, still reads.
    pub hook: String,
    /// How many requests the percentiles cover: the hook's latest, as many as the daemon keeps.
    pub requests: u64,
    /// The median.
    pub p50_us: u64,
    /// The 95th percentile.
    pub p95_us: u64,
    /// The 99th percentile.
    pub p99_us: u64,
}

/// An error a daemon met, in words without file contents, prompt text or tool output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentError {
    /// When it last happened, in UTC milliseconds.
    pub at_ms: i64,
    /// How many times it happened while the daemon kept it.
    pub count: u64,
    /// What failed and why.
    pub message: String,
}
