//! Lists of what agents did, as receipts read them: the sessions that started in a span of time,
//! the subagents a session started, a session's tasks, how many tool calls each task made and each
//! session made outside its tasks, how many times a session compacted, and how many tasks were held
//! out from Cortex.

use std::collections::BTreeMap;

use cortex_core::{Identifier, Ulid};
use rusqlite::Params;

use crate::activity::{RawSession, RawTask, SessionRow, TaskRow};
use crate::error::{StoreError, sqlite};
use crate::queries::{Queries, json_array};
use crate::schema;

impl Queries<'_> {
    /// The sessions that started at or after `since_ms`, subagents' among them, by start time and
    /// then by id.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn sessions_started_since(self, since_ms: i64) -> Result<Vec<SessionRow>, StoreError> {
        self.session_rows(schema::SESSIONS_STARTED_SINCE, [since_ms])
    }

    /// The sessions `session_id` started: its subagents, by start time and then by id.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn sessions_started_by(
        self,
        session_id: &Identifier,
    ) -> Result<Vec<SessionRow>, StoreError> {
        self.session_rows(schema::SESSIONS_STARTED_BY, [session_id.as_str()])
    }

    /// The tasks of `session_id`, in the order they started.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn tasks_of_session(self, session_id: &Identifier) -> Result<Vec<TaskRow>, StoreError> {
        let raw: Vec<RawTask> = self
            .connection()
            .prepare_cached(schema::TASKS_OF_SESSION)
            .and_then(|mut statement| {
                statement
                    .query_map([session_id.as_str()], RawTask::read)?
                    .collect::<rusqlite::Result<_>>()
            })
            .map_err(sqlite("reading a session's tasks"))?;
        raw.into_iter().map(RawTask::check).collect()
    }

    /// How many tool calls each of `tasks` made, its subagents' among them, for each task that made
    /// any. A call is one tool's report through a hook: the rows it was stored as, one for each
    /// stretch of a file it touched, count once, and the edits the watcher saw count not at all.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn tool_calls_of_tasks(self, tasks: &[Ulid]) -> Result<BTreeMap<Ulid, u64>, StoreError> {
        // Task ids are Crockford base32: quoted, they need no escaping.
        let listed = json_array(tasks.iter().map(|task| format!("\"{task}\"")));
        let counted: Vec<(String, i64, i64)> = self
            .connection()
            .prepare_cached(schema::TOOL_CALLS_OF_TASKS)
            .and_then(|mut statement| {
                statement
                    .query_map([listed], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<rusqlite::Result<_>>()
            })
            .map_err(sqlite("counting tool calls"))?;
        counted
            .into_iter()
            .map(|(task, calls, row)| {
                let corrupt = || StoreError::Corrupt {
                    column: "events.task_id",
                    row,
                };
                let task = Ulid::parse(&task).map_err(|_| corrupt())?;
                Ok((task, u64::try_from(calls).map_err(|_| corrupt())?))
            })
            .collect()
    }

    /// How many tool calls `session_id` made while it had no task: before its first prompt, or
    /// after a stop and before the next prompt. Counted as [`Queries::tool_calls_of_tasks`] counts.
    ///
    /// # Errors
    ///
    /// SQLite errors and a count that is not one.
    pub fn tool_calls_outside_tasks(self, session_id: &Identifier) -> Result<u64, StoreError> {
        let calls: i64 = self
            .connection()
            .prepare_cached(schema::TOOL_CALLS_OUTSIDE_TASKS)
            .and_then(|mut statement| statement.query_row([session_id.as_str()], |row| row.get(0)))
            .map_err(sqlite("counting tool calls outside tasks"))?;
        u64::try_from(calls).map_err(|_| StoreError::Corrupt {
            column: "events.task_id",
            row: calls,
        })
    }

    /// When `session_id` compacted its conversation, in UTC milliseconds, earliest first: the
    /// times its agent's hooks reported each compaction before it began.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn compactions_of_session(self, session_id: &Identifier) -> Result<Vec<i64>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::COMPACTIONS_OF_SESSION)
            .map_err(sqlite("reading compactions"))?;
        statement
            .query_map([session_id.as_str()], |row| row.get(0))
            .and_then(Iterator::collect)
            .map_err(sqlite("reading compactions"))
    }

    /// How many of the repository's tasks were held out from Cortex and how many were not, among
    /// those it has decided for, whenever they ran.
    ///
    /// # Errors
    ///
    /// SQLite errors and a count that is not one.
    pub fn holdout_arms(self) -> Result<(u64, u64), StoreError> {
        let (held_out, treated): (i64, i64) = self
            .connection()
            .prepare_cached(schema::HOLDOUT_ARMS)
            .and_then(|mut statement| statement.query_row([], |row| Ok((row.get(0)?, row.get(1)?))))
            .map_err(sqlite("counting held-out tasks"))?;
        let count = |value: i64| {
            u64::try_from(value).map_err(|_| StoreError::Corrupt {
                column: "tasks.holdout",
                row: value,
            })
        };
        Ok((count(held_out)?, count(treated)?))
    }

    /// The sessions one statement finds with `params`.
    fn session_rows(
        self,
        sql: &'static str,
        params: impl Params,
    ) -> Result<Vec<SessionRow>, StoreError> {
        let raw: Vec<RawSession> = self
            .connection()
            .prepare_cached(sql)
            .and_then(|mut statement| {
                statement
                    .query_map(params, RawSession::read)?
                    .collect::<rusqlite::Result<_>>()
            })
            .map_err(sqlite("reading sessions"))?;
        raw.into_iter().map(RawSession::check).collect()
    }
}
