//! Sessions, tasks and events: what agents did in the repository.
//!
//! Each row keeps ids, fixed names of kinds, paths, line ranges, times, sizes and exit codes. The
//! prompt that starts a task is kept as its hash, and as an excerpt only when the user asks; the
//! records have no field for anything else an agent wrote or a tool printed.

use std::path::PathBuf;

use cortex_core::{
    Agent, ContentHash, EventKind, EventMeta, Identifier, LineRange, PromptExcerpt, RepoPath,
    SessionStatus, TaskOutcome, Ulid,
};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::Queries;
use crate::schema;
use crate::values;

/// A session as its agent describes it when it starts or is first seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    /// The session's id, as the agent names it.
    pub session_id: Identifier,
    /// The agent.
    pub agent: Agent,
    /// The agent's version, when known.
    pub agent_version: Option<Identifier>,
    /// The model, when known.
    pub model: Option<Identifier>,
    /// The session that started this one, for a subagent.
    pub parent_session_id: Option<Identifier>,
    /// The agent's working folder.
    pub cwd: Option<PathBuf>,
    /// The agent's transcript of the session.
    pub transcript_path: Option<PathBuf>,
    /// When the session started, in UTC milliseconds.
    pub started_at_ms: i64,
}

/// A task as a prompt starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    /// The task's id.
    pub task_id: Ulid,
    /// The session the prompt came in.
    pub session_id: Identifier,
    /// When the prompt came, in UTC milliseconds.
    pub started_at_ms: i64,
    /// The hash of the prompt's text.
    pub prompt_hash: Option<ContentHash>,
    /// The prompt's first characters, only when the user asks for them.
    pub prompt_excerpt: Option<PromptExcerpt>,
}

/// One event to append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    /// The task it belongs to, if any.
    pub task_id: Option<Ulid>,
    /// The session that reported it, if any.
    pub session_id: Option<Identifier>,
    /// When it happened, in UTC milliseconds.
    pub ts_ms: i64,
    /// What it records.
    pub kind: EventKind,
    /// The graph node of its file or folder, when indexed.
    pub node_id: Option<NodeId>,
    /// Its file or folder.
    pub path: Option<RepoPath>,
    /// The lines it covers.
    pub range: Option<LineRange>,
    /// The other facts it keeps.
    pub meta: EventMeta,
}

/// A stored session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// What was recorded about it.
    pub record: SessionRecord,
    /// When it ended, if it has.
    pub ended_at_ms: Option<i64>,
    /// Whether it still runs.
    pub status: SessionStatus,
}

/// A stored task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    /// What was recorded when it started.
    pub record: TaskRecord,
    /// Its number in the repository's order of tasks, from 1.
    pub seq: u64,
    /// When it ended, if it has.
    pub ended_at_ms: Option<i64>,
    /// Whether its injections were held back, once decided.
    pub holdout: Option<bool>,
    /// How it ended, once known.
    pub outcome: Option<TaskOutcome>,
}

/// A stored event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    /// Its number: events are numbered in the order they were stored.
    pub seq: u64,
    /// What was recorded.
    pub record: EventRecord,
}

/// Adds the session, or updates a stored one: facts given replace those stored, facts not given
/// keep theirs, the first start time stays, and the session is live again.
pub(crate) fn record_session(
    connection: &Connection,
    session: &SessionRecord,
) -> Result<(), StoreError> {
    let cwd = text_of(session.cwd.as_ref(), "sessions.cwd")?;
    let transcript = text_of(session.transcript_path.as_ref(), "sessions.transcript_path")?;
    connection
        .prepare_cached(schema::SESSION_UPSERT)
        .and_then(|mut statement| {
            statement.execute((
                session.session_id.as_str(),
                session.agent.name(),
                session.agent_version.as_ref().map(Identifier::as_str),
                session.model.as_ref().map(Identifier::as_str),
                session.parent_session_id.as_ref().map(Identifier::as_str),
                cwd,
                transcript,
                session.started_at_ms,
                SessionStatus::Live.name(),
            ))
        })
        .map(drop)
        .map_err(sqlite("writing a session"))
}

/// Marks the session ended at `ended_at_ms`. `false` when no such session is stored.
pub(crate) fn end_session(
    connection: &Connection,
    session_id: &Identifier,
    ended_at_ms: i64,
) -> Result<bool, StoreError> {
    connection
        .prepare_cached(schema::SESSION_END)
        .and_then(|mut statement| {
            statement.execute((
                session_id.as_str(),
                ended_at_ms,
                SessionStatus::Ended.name(),
            ))
        })
        .map(|rows| rows > 0)
        .map_err(sqlite("ending a session"))
}

/// Adds the task with the next number after every task stored, and returns that number.
pub(crate) fn start_task(connection: &Connection, task: &TaskRecord) -> Result<u64, StoreError> {
    let seq: i64 = connection
        .prepare_cached(schema::TASK_INSERT)
        .and_then(|mut statement| {
            statement.query_row(
                (
                    task.task_id.to_string(),
                    task.session_id.as_str(),
                    task.started_at_ms,
                    task.prompt_hash.as_ref().map(|hash| *hash.as_bytes()),
                    task.prompt_excerpt.as_ref().map(PromptExcerpt::as_str),
                ),
                |row| row.get(0),
            )
        })
        .map_err(sqlite("writing a task"))?;
    values::number(seq, "tasks.seq", seq)
}

/// Ends the task with `outcome` at `ended_at_ms`. `false` when no such task is open.
pub(crate) fn end_task(
    connection: &Connection,
    task_id: Ulid,
    ended_at_ms: i64,
    outcome: TaskOutcome,
) -> Result<bool, StoreError> {
    connection
        .prepare_cached(schema::TASK_END)
        .and_then(|mut statement| {
            statement.execute((task_id.to_string(), ended_at_ms, outcome.name()))
        })
        .map(|rows| rows > 0)
        .map_err(sqlite("ending a task"))
}

/// Appends the event and returns its number.
pub(crate) fn append_event(
    connection: &Connection,
    event: &EventRecord,
) -> Result<u64, StoreError> {
    let meta = serde_json::to_string(&event.meta).map_err(|_| StoreError::NotText {
        column: "events.meta",
    })?;
    let seq: i64 = connection
        .prepare_cached(schema::EVENT_INSERT)
        .and_then(|mut statement| {
            statement.query_row(
                (
                    event.task_id.map(|id| id.to_string()),
                    event.session_id.as_ref().map(Identifier::as_str),
                    event.ts_ms,
                    event.kind.name(),
                    event.node_id.map(NodeId::get),
                    event.path.as_ref().map(RepoPath::as_str),
                    event.range.map(LineRange::start_line),
                    event.range.map(LineRange::end_line),
                    meta,
                ),
                |row| row.get(0),
            )
        })
        .map_err(sqlite("writing an event"))?;
    values::number(seq, "events.seq", seq)
}

/// `path` as text for `column`.
fn text_of<'p>(
    path: Option<&'p PathBuf>,
    column: &'static str,
) -> Result<Option<&'p str>, StoreError> {
    path.map(|path| path.to_str().ok_or(StoreError::NotText { column }))
        .transpose()
}

impl Queries<'_> {
    /// The session with id `session_id`.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn session(self, session_id: &Identifier) -> Result<Option<SessionRow>, StoreError> {
        self.connection()
            .prepare_cached(schema::SESSION_BY_ID)
            .and_then(|mut statement| {
                statement
                    .query_row([session_id.as_str()], RawSession::read)
                    .optional()
            })
            .map_err(sqlite("reading a session"))?
            .map(RawSession::check)
            .transpose()
    }

    /// The task with id `task_id`.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn task(self, task_id: Ulid) -> Result<Option<TaskRow>, StoreError> {
        self.task_row(schema::TASK_BY_ID, &task_id.to_string(), "reading a task")
    }

    /// The latest task of the session that has not ended.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn open_task(self, session_id: &Identifier) -> Result<Option<TaskRow>, StoreError> {
        self.task_row(
            schema::TASK_OPEN_OF_SESSION,
            session_id.as_str(),
            "reading a session's open task",
        )
    }

    /// The task one statement finds by one text parameter.
    fn task_row(
        self,
        sql: &'static str,
        key: &str,
        operation: &'static str,
    ) -> Result<Option<TaskRow>, StoreError> {
        self.connection()
            .prepare_cached(sql)
            .and_then(|mut statement| statement.query_row([key], RawTask::read).optional())
            .map_err(sqlite(operation))?
            .map(RawTask::check)
            .transpose()
    }

    /// The events of the task, in the order they were stored.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn events_of_task(self, task_id: Ulid) -> Result<Vec<EventRow>, StoreError> {
        self.event_rows(schema::EVENTS_OF_TASK, &task_id.to_string())
    }

    /// The events the session reported, by time, then in the order they were stored.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn events_of_session(self, session_id: &Identifier) -> Result<Vec<EventRow>, StoreError> {
        self.event_rows(schema::EVENTS_OF_SESSION, session_id.as_str())
    }

    /// The events numbered after `after_seq`, in order, at most `limit` of them.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn events_after(self, after_seq: u64, limit: usize) -> Result<Vec<EventRow>, StoreError> {
        let after = values::integer(after_seq, "events.seq")?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let raw: Vec<RawEvent> = self
            .connection()
            .prepare_cached(schema::EVENTS_AFTER)
            .and_then(|mut statement| {
                statement
                    .query_map((after, limit), RawEvent::read)?
                    .collect::<rusqlite::Result<_>>()
            })
            .map_err(sqlite("reading events"))?;
        raw.into_iter().map(RawEvent::check).collect()
    }

    /// The events one statement finds by one text parameter.
    fn event_rows(self, sql: &'static str, key: &str) -> Result<Vec<EventRow>, StoreError> {
        let raw: Vec<RawEvent> = self
            .connection()
            .prepare_cached(sql)
            .and_then(|mut statement| {
                statement
                    .query_map([key], RawEvent::read)?
                    .collect::<rusqlite::Result<_>>()
            })
            .map_err(sqlite("reading events"))?;
        raw.into_iter().map(RawEvent::check).collect()
    }
}

/// A `sessions` row as SQLite returns it.
struct RawSession {
    /// `rowid`, for reporting a damaged row.
    rowid: i64,
    /// `session_id`.
    session_id: String,
    /// `agent`.
    agent: String,
    /// `agent_version`.
    agent_version: Option<String>,
    /// `model`.
    model: Option<String>,
    /// `parent_session_id`.
    parent_session_id: Option<String>,
    /// `cwd`.
    cwd: Option<String>,
    /// `transcript_path`.
    transcript_path: Option<String>,
    /// `started_at`.
    started_at: i64,
    /// `ended_at`.
    ended_at: Option<i64>,
    /// `status`.
    status: String,
}

impl RawSession {
    /// Reads the columns of [`schema::SESSION_BY_ID`].
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            rowid: row.get(0)?,
            session_id: row.get(1)?,
            agent: row.get(2)?,
            agent_version: row.get(3)?,
            model: row.get(4)?,
            parent_session_id: row.get(5)?,
            cwd: row.get(6)?,
            transcript_path: row.get(7)?,
            started_at: row.get(8)?,
            ended_at: row.get(9)?,
            status: row.get(10)?,
        })
    }

    /// The typed row, or [`StoreError::Corrupt`] naming the first column that cannot be read.
    fn check(self) -> Result<SessionRow, StoreError> {
        let row = self.rowid;
        let name = |text: Option<String>, column| {
            text.map(|text| values::identifier(&text, column, row))
                .transpose()
        };
        Ok(SessionRow {
            record: SessionRecord {
                session_id: values::identifier(&self.session_id, "sessions.session_id", row)?,
                agent: values::kind(&self.agent, Agent::from_name, "sessions.agent", row)?,
                agent_version: name(self.agent_version, "sessions.agent_version")?,
                model: name(self.model, "sessions.model")?,
                parent_session_id: name(self.parent_session_id, "sessions.parent_session_id")?,
                cwd: self.cwd.map(PathBuf::from),
                transcript_path: self.transcript_path.map(PathBuf::from),
                started_at_ms: self.started_at,
            },
            ended_at_ms: self.ended_at,
            status: values::kind(
                &self.status,
                SessionStatus::from_name,
                "sessions.status",
                row,
            )?,
        })
    }
}

/// A `tasks` row as SQLite returns it.
struct RawTask {
    /// `rowid`, for reporting a damaged row.
    rowid: i64,
    /// `task_id`.
    task_id: String,
    /// `session_id`.
    session_id: String,
    /// `seq`.
    seq: i64,
    /// `started_at`.
    started_at: i64,
    /// `ended_at`.
    ended_at: Option<i64>,
    /// `prompt_hash`.
    prompt_hash: Option<[u8; 32]>,
    /// `prompt_excerpt`.
    prompt_excerpt: Option<String>,
    /// `holdout`.
    holdout: Option<i64>,
    /// `outcome`.
    outcome: Option<String>,
}

impl RawTask {
    /// Reads the columns of [`schema::TASK_BY_ID`].
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            rowid: row.get(0)?,
            task_id: row.get(1)?,
            session_id: row.get(2)?,
            seq: row.get(3)?,
            started_at: row.get(4)?,
            ended_at: row.get(5)?,
            prompt_hash: row.get(6)?,
            prompt_excerpt: row.get(7)?,
            holdout: row.get(8)?,
            outcome: row.get(9)?,
        })
    }

    /// The typed row, or [`StoreError::Corrupt`] naming the first column that cannot be read.
    fn check(self) -> Result<TaskRow, StoreError> {
        let row = self.rowid;
        let corrupt = |column| StoreError::Corrupt { column, row };
        let excerpt = self
            .prompt_excerpt
            .map(|text| PromptExcerpt::stored(&text).ok_or(corrupt("tasks.prompt_excerpt")))
            .transpose()?;
        let holdout = self
            .holdout
            .map(|flag| match flag {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(corrupt("tasks.holdout")),
            })
            .transpose()?;
        let outcome = self
            .outcome
            .map(|name| values::kind(&name, TaskOutcome::from_name, "tasks.outcome", row))
            .transpose()?;
        Ok(TaskRow {
            record: TaskRecord {
                task_id: Ulid::parse(&self.task_id).map_err(|_| corrupt("tasks.task_id"))?,
                session_id: values::identifier(&self.session_id, "tasks.session_id", row)?,
                started_at_ms: self.started_at,
                prompt_hash: self.prompt_hash.map(ContentHash::from_bytes),
                prompt_excerpt: excerpt,
            },
            seq: values::number(self.seq, "tasks.seq", row)?,
            ended_at_ms: self.ended_at,
            holdout,
            outcome,
        })
    }
}

/// An `events` row as SQLite returns it.
struct RawEvent {
    /// `seq`.
    seq: i64,
    /// `task_id`.
    task_id: Option<String>,
    /// `session_id`.
    session_id: Option<String>,
    /// `ts`.
    ts: i64,
    /// `kind`.
    kind: String,
    /// `node_id`.
    node_id: Option<i64>,
    /// `path`.
    path: Option<String>,
    /// `range_start`.
    range_start: Option<i64>,
    /// `range_end`.
    range_end: Option<i64>,
    /// `meta`.
    meta: String,
}

impl RawEvent {
    /// Reads the columns of [`schema::EVENTS_OF_TASK`].
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            seq: row.get(0)?,
            task_id: row.get(1)?,
            session_id: row.get(2)?,
            ts: row.get(3)?,
            kind: row.get(4)?,
            node_id: row.get(5)?,
            path: row.get(6)?,
            range_start: row.get(7)?,
            range_end: row.get(8)?,
            meta: row.get(9)?,
        })
    }

    /// The typed row, or [`StoreError::Corrupt`] naming the first column that cannot be read.
    fn check(self) -> Result<EventRow, StoreError> {
        let row = self.seq;
        let corrupt = |column| StoreError::Corrupt { column, row };
        let task_id = self
            .task_id
            .map(|text| Ulid::parse(&text).map_err(|_| corrupt("events.task_id")))
            .transpose()?;
        let session_id = self
            .session_id
            .map(|text| values::identifier(&text, "events.session_id", row))
            .transpose()?;
        let range = match (self.range_start, self.range_end) {
            (None, None) => None,
            (Some(start), Some(end)) => {
                let start = values::number(start, "events.range_start", row)?;
                let end = values::number(end, "events.range_end", row)?;
                Some(LineRange::new(start, end).map_err(|_| corrupt("events.range_end"))?)
            }
            _ => return Err(corrupt("events.range_end")),
        };
        Ok(EventRow {
            seq: values::number(self.seq, "events.seq", row)?,
            record: EventRecord {
                task_id,
                session_id,
                ts_ms: self.ts,
                kind: values::kind(&self.kind, EventKind::from_name, "events.kind", row)?,
                node_id: self.node_id.map(NodeId),
                path: self
                    .path
                    .map(|text| values::path(&text, "events.path", row))
                    .transpose()?,
                range,
                meta: serde_json::from_str(&self.meta).map_err(|_| corrupt("events.meta"))?,
            },
        })
    }
}
