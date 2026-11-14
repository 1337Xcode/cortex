//! Handoffs: what a session that stopped in the middle of a task leaves for the next session to
//! pick it up. A handoff names the files the task edited and read (graph nodes), the warnings its
//! edits got that were not resolved, and the last check command it ran when that one failed (its
//! runner, kind and exit code). Never code, prompt text or a command line. Each session keeps at
//! most one open handoff, its latest; one is closed by the session it is given to, or by a later
//! success in the session that left it.

use cortex_core::{CheckCounts, CommandKind, Identifier, Ulid};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{StoreError, sqlite};
use crate::ids::{HandoffId, NodeId, WarningId};
use crate::queries::Queries;
use crate::schema;

/// A check command that failed: what ran it and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastFailure {
    /// The runner, by the name Cortex gives it (`pytest`, `cargo`).
    pub runner: Identifier,
    /// What it checks.
    pub kind: CommandKind,
    /// Its exit code.
    pub exit_code: i32,
    /// The counts its output printed, when they were read.
    pub counts: Option<CheckCounts>,
    /// Its command, cleaned of what must not be kept (`cortex_learn::clean_command`), when it
    /// could be: kept until the handoff closes or a day passes.
    pub command: Option<String>,
    /// The test file it ran, when its command could not be kept.
    pub test_path: Option<String>,
}

/// How the task a handoff was left by ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffEnd {
    /// Its answer ended, without a check that passed.
    Answered,
    /// It was left with no end of its answer: the session ended, or its next prompt came.
    Left,
    /// Its turn stopped on an error, such as a usage limit.
    Stopped,
}

/// What a session whose task did not succeed leaves.
// INVARIANT: no-content-stored (node ids, a runner's name, a check's kind, an exit code and its
// failure's cleaned command and parsed counts: no prompt text, raw command line or output)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffRecord {
    /// The session that left it.
    pub from_session: Identifier,
    /// The task it was in.
    pub task_id: Ulid,
    /// How that task ended.
    pub end: HandoffEnd,
    /// The files the task edited, in the order it first edited them.
    pub edited: Vec<NodeId>,
    /// The files it read and did not edit, in the order it first read them.
    pub read: Vec<NodeId>,
    /// The warnings its edits got that were not resolved.
    pub open_warnings: Vec<WarningId>,
    /// Its last check command, when that one failed.
    pub last_failure: Option<LastFailure>,
    /// When the task ended, in UTC milliseconds.
    pub created_at_ms: i64,
}

/// A handoff as the store holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffRow {
    /// Its row.
    pub id: HandoffId,
    /// What was left.
    pub record: HandoffRecord,
}

/// The files of a handoff as their column holds them.
#[derive(Serialize, Deserialize)]
struct WorkingSet {
    /// The edited files' node ids.
    edited: Vec<i64>,
    /// The read files' node ids.
    read: Vec<i64>,
    /// How the task ended; a row written before this was kept was left by a task left.
    #[serde(default = "left")]
    end: HandoffEnd,
}

/// The end of a task whose handoff row does not say: only tasks left wrote handoffs then.
fn left() -> HandoffEnd {
    HandoffEnd::Left
}

/// A failure as its column holds it.
#[derive(Serialize, Deserialize)]
struct Failure {
    /// The runner's name.
    runner: String,
    /// The check's kind, by its name.
    kind: String,
    /// The exit code.
    exit_code: i32,
    /// The counts its output printed; none in a row written before they were kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    counts: Option<CheckCounts>,
    /// The cleaned command; none when it could not be kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    /// The test file, when the command could not be kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    test_path: Option<String>,
}

/// Leaves `record` as its session's open handoff, in place of any it left before.
pub(crate) fn leave(
    connection: &Connection,
    record: &HandoffRecord,
) -> Result<HandoffId, StoreError> {
    let not_text = |column| move |_| StoreError::NotText { column };
    let working_set = serde_json::to_string(&WorkingSet {
        edited: record.edited.iter().map(|node| node.get()).collect(),
        read: record.read.iter().map(|node| node.get()).collect(),
        end: record.end,
    })
    .map_err(not_text("handoffs.working_set"))?;
    let warnings: Vec<i64> = record.open_warnings.iter().map(|id| id.get()).collect();
    let warnings = serde_json::to_string(&warnings).map_err(not_text("handoffs.open_warnings"))?;
    let failure = record
        .last_failure
        .as_ref()
        .map(|failure| {
            serde_json::to_string(&Failure {
                runner: failure.runner.as_str().to_owned(),
                kind: failure.kind.name().to_owned(),
                exit_code: failure.exit_code,
                counts: failure.counts,
                command: failure.command.clone(),
                test_path: failure.test_path.clone(),
            })
        })
        .transpose()
        .map_err(not_text("handoffs.last_failure"))?;
    let session = record.from_session.as_str();
    connection
        .prepare_cached(schema::HANDOFFS_REPLACE_OF_SESSION)
        .and_then(|mut statement| statement.execute([session]))
        .map_err(sqlite("replacing a session's handoff"))?;
    connection
        .prepare_cached(schema::HANDOFF_LEAVE)
        .and_then(|mut statement| {
            statement.insert((
                session,
                record.task_id.to_string(),
                working_set,
                warnings,
                failure,
                record.created_at_ms,
            ))
        })
        .map(HandoffId)
        .map_err(sqlite("leaving a handoff"))
}

/// Closes handoff `id`, as picked up by session `by`: `false` when it was closed already, so that a
/// handoff given twice is closed once, by the first to record it.
pub(crate) fn close(
    connection: &Connection,
    id: HandoffId,
    by: &Identifier,
) -> Result<bool, StoreError> {
    connection
        .prepare_cached(schema::HANDOFF_CLOSE)
        .and_then(|mut statement| statement.execute((id.0, by.as_str())))
        .map(|rows| rows > 0)
        .map_err(sqlite("closing a handoff"))
}

impl Queries<'_> {
    /// The open handoffs left at `since_ms` or later, the newest first.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a row that is not what the store writes.
    pub fn open_handoffs_since(self, since_ms: i64) -> Result<Vec<HandoffRow>, StoreError> {
        self.handoffs(schema::HANDOFFS_OPEN_SINCE, since_ms)
    }

    /// The open handoffs session `session` left, the latest first.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a row that is not what the store writes.
    pub fn open_handoffs_of(self, session: &Identifier) -> Result<Vec<HandoffRow>, StoreError> {
        self.handoffs(schema::HANDOFFS_OPEN_OF_SESSION, session.as_str())
    }

    /// The handoffs statement `sql` finds with its one parameter `key`.
    fn handoffs(
        self,
        sql: &'static str,
        key: impl rusqlite::ToSql,
    ) -> Result<Vec<HandoffRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(sql)
            .map_err(sqlite("reading handoffs"))?;
        let rows = statement
            .query_map([key], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .map_err(sqlite("reading handoffs"))?;
        let mut handoffs = Vec::new();
        for row in rows {
            handoffs.push(row_of(row.map_err(sqlite("reading handoffs"))?)?);
        }
        Ok(handoffs)
    }
}

/// A handoff's columns, as read.
type Columns = (i64, String, String, String, String, Option<String>, i64);

/// The handoff `columns` hold.
fn row_of(
    (id, session, task, working_set, warnings, failure, created_at_ms): Columns,
) -> Result<HandoffRow, StoreError> {
    let corrupt = |column: &'static str| StoreError::Corrupt { column, row: id };
    let from_session = Identifier::new(&session).map_err(|_| corrupt("handoffs.from_session"))?;
    let task_id = Ulid::parse(&task).map_err(|_| corrupt("handoffs.task_id"))?;
    let set: WorkingSet =
        serde_json::from_str(&working_set).map_err(|_| corrupt("handoffs.working_set"))?;
    let warnings: Vec<i64> =
        serde_json::from_str(&warnings).map_err(|_| corrupt("handoffs.open_warnings"))?;
    let last_failure = failure
        .map(|text| {
            let failure: Failure =
                serde_json::from_str(&text).map_err(|_| corrupt("handoffs.last_failure"))?;
            let runner =
                Identifier::new(&failure.runner).map_err(|_| corrupt("handoffs.last_failure"))?;
            let kind = CommandKind::from_name(&failure.kind).ok_or(StoreError::Corrupt {
                column: "handoffs.last_failure",
                row: id,
            })?;
            Ok(LastFailure {
                runner,
                kind,
                exit_code: failure.exit_code,
                counts: failure.counts,
                command: failure.command,
                test_path: failure.test_path,
            })
        })
        .transpose()?;
    Ok(HandoffRow {
        id: HandoffId(id),
        record: HandoffRecord {
            from_session,
            task_id,
            end: set.end,
            edited: set.edited.into_iter().map(NodeId).collect(),
            read: set.read.into_iter().map(NodeId).collect(),
            open_warnings: warnings.into_iter().map(WarningId).collect(),
            last_failure,
            created_at_ms,
        },
    })
}

/// Deletes every handoff left before `before_ms`, open or closed: how many.
pub(crate) fn delete_before(connection: &Connection, before_ms: i64) -> Result<u64, StoreError> {
    connection
        .prepare_cached(schema::HANDOFFS_DELETE_BEFORE)
        .and_then(|mut statement| statement.execute([before_ms]))
        .map(|deleted| u64::try_from(deleted).unwrap_or(u64::MAX))
        .map_err(sqlite("deleting old handoffs"))
}

/// Clears the last failure of every handoff left before `before_ms`: how many.
pub(crate) fn forget_failures(connection: &Connection, before_ms: i64) -> Result<u64, StoreError> {
    connection
        .prepare_cached(schema::HANDOFFS_FORGET_FAILURES_BEFORE)
        .and_then(|mut statement| statement.execute([before_ms]))
        .map(|cleared| u64::try_from(cleared).unwrap_or(u64::MAX))
        .map_err(sqlite("forgetting old handoffs' failures"))
}
