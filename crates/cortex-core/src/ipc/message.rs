//! The requests a hook or the CLI sends a daemon, and the daemon's responses.
//!
//! Every frame is an envelope holding a `request_id` and a `body` tagged by its `type`. The first
//! request on a connection is a Hello. The envelope, Hello, Mismatch and Shutdown never change
//! shape, so a process of any version can tell a daemon of any other version to stop.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::claims::{ClaimsAnswer, ClaimsAsk};
use super::facts::{FactsAnswer, FactsAsk, Note};
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
    /// Record a prompt's event, and answer with the work order the prompt gets, if any, and the
    /// coordination notices waiting for its session; a prompt that asks for no code (a slash
    /// command, "ok") gets only the notices.
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
    /// Record a tool's result, and answer with the warnings it gets if it changed a file, and the
    /// coordination notices waiting for its session (protocol 3.0 added it, for edits).
    PostEdit {
        /// The tool's result.
        event: Box<Event>,
        /// When the hook stops reading the answer, in UTC milliseconds by the wall clock, as for
        /// `Retrieve`: warnings that cannot reach the hook by then are not sent, and not recorded
        /// as given. None from a client that waits as long as it takes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer_by_ms: Option<i64>,
    },
    /// Answer the moment before a tool runs with the hint it gets, if any, recording nothing: a
    /// read's hint before a read, and before a shell command what the same command gave when it ran
    /// earlier in the session (protocol 4.0 added it, for reads; the name stays, as it is the
    /// message's tag on the wire).
    ReadHint {
        /// The tool's event: the moment before a tool reads a file or runs a shell command.
        event: Box<Event>,
        /// When the hook stops reading the answer, in UTC milliseconds by the wall clock, as for
        /// `Retrieve`: a hint that cannot reach the hook by then is not sent. None from a client
        /// that waits as long as it takes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer_by_ms: Option<i64>,
    },
    /// Record an agent's stop, and answer with the reason strict mode gives it to go on, if any
    /// (protocol 5.0 added it).
    StopReason {
        /// The stop's event.
        event: Box<Event>,
        /// When the hook stops reading the answer, in UTC milliseconds by the wall clock, as for
        /// `Retrieve`: a reason that cannot reach the hook by then is not sent. None from a client
        /// that waits as long as it takes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer_by_ms: Option<i64>,
    },
    /// Record a compaction about to run, and answer with the keep-list its summary is given, if
    /// any: what the session's open task did, for the summary to keep (protocol 11.0 added it).
    KeepList {
        /// The compaction's event.
        event: Box<Event>,
        /// When the hook stops reading the answer, in UTC milliseconds by the wall clock, as for
        /// `Retrieve`: a keep-list that cannot reach the hook by then is not sent, and recorded as
        /// not delivered. None from a client that waits as long as it takes.
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
        /// Asked from a linked working tree, its top, whose files the answer shows (protocol 11.1
        /// added it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_root: Option<PathBuf>,
    },
    /// Answer why a file or a symbol is where it is, as `cortex why` asks: what its file is linked
    /// to and how strongly, the facts about it, the sessions on its file now and the warnings
    /// about it (protocol 8.0 added it).
    Why {
        /// A repository path, that path and `:line` or `:symbol`, or a symbol's name.
        target: String,
        /// Asked from a linked working tree, its top, whose files the answer checks facts against
        /// (protocol 11.1 added it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_root: Option<PathBuf>,
    },
    /// Note a fact of the user's, and answer with what became of it (protocol 6.0 added it).
    Note(Note),
    /// List the facts, or carry the user's word on one, and answer with the outcome (protocol 6.0
    /// added it).
    Facts(FactsAsk),
    /// List the claims holding now, claim files for the session at work, or release claims
    /// (protocol 9.0 added it).
    Claims(ClaimsAsk),
    /// Run upkeep as soon as the writer is free, answered at once (protocol 10.0 added it).
    Consolidate,
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
#[allow(
    clippy::large_enum_variant,
    reason = "a response is built once for its request and moved once into its frame; boxing the \
              status would add an allocation and change every match on it for nothing"
)]
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
        /// A message for the user and never the model, as when a step the daemon runs on its own
        /// failed. Absent from the frame when there is none, so a hook built before it reads the
        /// reply as before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_user: Option<String>,
    },
    /// What became of a note or of the user's word on the facts, or the facts listed (protocol 6.0
    /// added it).
    Facts(FactsAnswer),
    /// The claims listed, or what became of a claim made or released (protocol 9.0 added it).
    Claims(ClaimsAnswer),
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
    /// What the daemon read of the work tree's uncommitted changes; none before its first look
    /// ended, or from a daemon that does not say (protocol 2.2 added it).
    #[serde(default)]
    pub worktree: Option<WorktreeReport>,
    /// What the daemon read of each linked working tree an agent worked in since it started, by
    /// its top (protocol 11.1 added it).
    #[serde(default)]
    pub linked_worktrees: Vec<LinkedWorktreeReport>,
    /// What the last run of upkeep did; none before the first, or from a daemon that predates
    /// upkeep.
    #[serde(default)]
    pub upkeep: Option<UpkeepReport>,
    /// What the history worker holds and waits on; none while `history.why` and `history.seeds`
    /// are both off, or from a daemon that does not say (protocol 11.2 added it).
    #[serde(default)]
    pub history: Option<HistoryReport>,
}

/// What a daemon's history worker holds, which reads who last changed the lines Cortex may show
/// and what those commits were, for its status.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HistoryReport {
    /// The commits the store keeps a card of.
    pub cards: u64,
    /// The blames of line ranges the store keeps.
    pub cached_blames: u64,
    /// The jobs waiting for the worker.
    pub jobs_waiting: u64,
    /// The blames that failed since the daemon started (git did not end in time, say); each is
    /// not tried again until the daemon starts again.
    pub failed_blames: u64,
    /// Why git is not run in the repository for history, when it is not: its own configuration
    /// names a program git would run, say. Kept for the daemon's life.
    pub refused: Option<String>,
}

/// What a run of upkeep removed, and whether it finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpkeepReport {
    /// When it ended, in UTC milliseconds.
    pub at_ms: i64,
    /// Links of use removed, faded to nothing.
    pub links: u64,
    /// Nodes whose interest was removed, faded to nothing.
    pub interest: u64,
    /// Facts deleted, retired long ago.
    pub facts: u64,
    /// Events deleted, past their retention.
    pub events: u64,
    /// Episodes deleted, past their cap.
    pub episodes: u64,
    /// Positions removed, no longer holding.
    pub positions: u64,
    /// Free pages of the database given back to the file system (protocol 10.1 added it).
    #[serde(default)]
    pub pages: u64,
    /// Whether every job was done; otherwise the next run carries on.
    pub is_done: bool,
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

/// What a daemon read of one linked working tree's uncommitted changes (`git worktree add`), which
/// seed the retrievals of that working tree's prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedWorktreeReport {
    /// The linked working tree's top.
    pub root: PathBuf,
    /// What its looks read; none before its first look ended.
    #[serde(default)]
    pub worktree: Option<WorktreeReport>,
}

/// What a daemon read of its work tree's uncommitted changes, and how its looks went, for its
/// status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeReport {
    /// Whether the latest look read the changes, or why they are not known.
    #[serde(deserialize_with = "worktree_state_or_other")]
    pub state: WorktreeState,
    /// The files of the uncommitted diff kept to seed retrievals, as of the latest look: at most
    /// 10, those the index saw change last.
    pub uncommitted_files: u64,
    /// The files git listed as uncommitted at the latest look, whether they seed or not; 0 when
    /// the latest look listed too many to count.
    pub listed_files: u64,
    /// How many looks ended since the daemon started, the failed ones not among them.
    pub looks: u64,
    /// How many looks failed; each was tried again.
    pub failed_looks: u64,
}

named_kind! {
    /// Whether a daemon's latest look at its work tree read the uncommitted changes, or why they
    /// are not known.
    WorktreeState {
        /// It read them.
        Read => "read",
        /// No git program is on the `PATH`.
        NoGit => "no_git",
        /// The repository's root folder holds no git repository.
        NoRepository => "no_repository",
        /// The repository's own git configuration names a filter, whose command a look would run,
        /// so the work tree is not looked at.
        Refused => "refused",
        /// The repository's own git configuration includes another file, which git would read
        /// wherever it is, so the work tree is not looked at.
        Includes => "includes",
        /// The repository is a partial clone and git is older than 2.45, which cannot be told not
        /// to fetch what the clone left out, so the work tree is not looked at.
        WouldFetch => "would_fetch",
        /// More files changed than a look takes in: a mass change, of which none seeds.
        TooMany => "too_many",
        /// HEAD moved since the latest look, so what it read may no longer hold; another look
        /// follows.
        HeadMoved => "head_moved",
        /// The latest look failed; another follows.
        Failed => "failed",
        /// A state this version does not know.
        Other => "other",
    }
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
    named_or(deserializer, GitState::from_name, GitState::Other)
}

/// A work tree's state from its name, or [`WorktreeState::Other`] for a name this version does not
/// know, so that a newer daemon's status still reads.
fn worktree_state_or_other<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<WorktreeState, D::Error> {
    named_or(deserializer, WorktreeState::from_name, WorktreeState::Other)
}

/// The value `from_name` gives for the name `deserializer` reads, or `other` for a name it does not
/// know.
fn named_or<'de, D: serde::Deserializer<'de>, K>(
    deserializer: D,
    from_name: fn(&str) -> Option<K>,
    other: K,
) -> Result<K, D::Error> {
    let name = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
    Ok(from_name(&name).unwrap_or(other))
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
