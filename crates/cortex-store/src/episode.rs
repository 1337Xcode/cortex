//! One record of a task: what it read before it changed anything, what it changed, what it ran to
//! check the change, what went wrong on the way, and what became of it.
//!
//! Every part holds node ids, counts, line numbers, exit codes and the names Cortex knows runners
//! by, and nothing else: no prompt text, no command line, no code. Each part is stored as JSON in
//! a column of its own, and the four together are held under [`MAX_EPISODE_BYTES`], which the
//! store refuses to write past, so that one task cannot fill the database however much it did.

use cortex_core::{CommandKind, Identifier, TaskOutcome};
use rusqlite::{Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::Queries;
use crate::schema;

/// Most bytes an episode's four parts hold as JSON together. A record of one task, not a copy of
/// the work.
pub const MAX_EPISODE_BYTES: usize = 2 * 1024;

/// A node id as the number it is stored as. Only the store makes ids, so reading one back from
/// JSON lives here rather than on the id itself.
mod as_number {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::ids::NodeId;

    /// Writes `node` as its number. Takes it by reference because serde's `with` asks for that.
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's with attribute passes the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(node: &NodeId, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_i64(node.get())
    }

    /// Reads a node from the number it was written as.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<NodeId, D::Error> {
        i64::deserialize(input).map(NodeId)
    }
}

/// Node ids as the numbers they are stored as, for a list of them.
mod as_numbers {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::ids::NodeId;

    /// Writes each of `nodes` as its number.
    pub(super) fn serialize<S: Serializer>(nodes: &[NodeId], out: S) -> Result<S::Ok, S::Error> {
        let numbers: Vec<i64> = nodes.iter().map(|node| node.get()).collect();
        numbers.serialize(out)
    }

    /// Reads the nodes from the numbers they were written as.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        input: D,
    ) -> Result<Vec<NodeId>, D::Error> {
        Vec::<i64>::deserialize(input).map(|numbers| numbers.into_iter().map(NodeId).collect())
    }
}

/// A file the task read before it changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFile {
    /// The file's node.
    #[serde(with = "as_number")]
    pub node: NodeId,
    /// How many times the task read it before its first change.
    pub reads: u32,
}

/// A symbol the task changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedSymbol {
    /// The symbol's node.
    #[serde(with = "as_number")]
    pub node: NodeId,
    /// Whether a change covered the line its declaration starts on, which is as near as the
    /// record comes to saying that its signature changed.
    pub header: bool,
}

/// A file the task changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// The file's node.
    #[serde(with = "as_number")]
    pub node: NodeId,
    /// How many of its lines the task's changes covered.
    pub lines: u32,
    /// The symbols among them.
    pub symbols: Vec<ChangedSymbol>,
}

/// A check the task ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RanCheck {
    /// The runner, by the name Cortex knows it by.
    pub runner: Identifier,
    /// What it checked.
    pub kind: CommandKind,
    /// How it ended, when it finished.
    pub code: Option<i32>,
}

/// A stretch of a file the task read over and over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadAgain {
    /// The file's node.
    #[serde(with = "as_number")]
    pub node: NodeId,
    /// The first line of the stretch.
    pub from_line: u32,
    /// The last line of the stretch.
    pub to_line: u32,
    /// How many times the task read it.
    pub times: u32,
}

/// What went wrong on the task's way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pitfalls {
    /// The runners whose check failed and later passed.
    pub fixed: Vec<Identifier>,
    /// The files the task changed and then put back itself.
    #[serde(with = "as_numbers")]
    pub undone: Vec<NodeId>,
    /// The stretches it read more times than a reader needs to.
    pub read_again: Vec<ReadAgain>,
}

/// One task's record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeRecord {
    /// The task.
    pub task_id: cortex_core::Ulid,
    /// What it read before it changed anything, in the order it first read each file.
    pub localize: Vec<ReadFile>,
    /// What it changed.
    pub change: Vec<ChangedFile>,
    /// The checks it ran, in order.
    pub verify: Vec<RanCheck>,
    /// What went wrong on the way.
    pub pitfalls: Pitfalls,
    /// What became of it.
    pub outcome: TaskOutcome,
    /// When the record was written, in UTC milliseconds.
    pub created_at_ms: i64,
}

impl EpisodeRecord {
    /// The record's four parts as the JSON they are stored as, in column order.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotText`] for a part that cannot be written as JSON.
    pub fn parts(&self) -> Result<[String; 4], StoreError> {
        Ok([
            json(&self.localize, "episodes.localize")?,
            json(&self.change, "episodes.change")?,
            json(&self.verify, "episodes.verify")?,
            json(&self.pitfalls, "episodes.pitfalls")?,
        ])
    }

    /// How many bytes the record's four parts hold as JSON.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotText`] for a part that cannot be written as JSON.
    pub fn size_bytes(&self) -> Result<usize, StoreError> {
        Ok(self.parts()?.iter().map(String::len).sum())
    }
}

/// Writes `episode`, replacing the task's record if it has one.
///
/// # Errors
///
/// [`StoreError::TooLarge`] for a record past [`MAX_EPISODE_BYTES`], which is the caller's to keep
/// to, [`StoreError::NotText`] for a part that cannot be written as JSON, and SQLite errors.
pub(crate) fn replace(connection: &Connection, episode: &EpisodeRecord) -> Result<(), StoreError> {
    let parts = episode.parts()?;
    let size_bytes: usize = parts.iter().map(String::len).sum();
    // INVARIANT: budgeted (an episode's parts hold at most MAX_EPISODE_BYTES)
    if size_bytes > MAX_EPISODE_BYTES {
        return Err(StoreError::TooLarge {
            column: "episodes",
            value: u64::try_from(size_bytes).unwrap_or(u64::MAX),
        });
    }
    let [localize, change, verify, pitfalls] = parts;
    connection
        .prepare_cached(schema::EPISODE_REPLACE)
        .and_then(|mut statement| {
            statement.execute((
                episode.task_id.to_string(),
                localize,
                change,
                verify,
                pitfalls,
                episode.outcome.name(),
                episode.created_at_ms,
            ))
        })
        .map(drop)
        .map_err(sqlite("writing an episode"))
}

impl Queries<'_> {
    /// The records of the oldest `limit` tasks, by task. The clustering reads them in this
    /// order, so the oldest task of a cluster is the one the others are compared against.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn episodes(self, limit: usize) -> Result<Vec<EpisodeRecord>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self
            .connection()
            .prepare_cached(schema::EPISODES)
            .map_err(sqlite("reading episodes"))?;
        let rows = statement
            .query_map([limit], |row| {
                let task_id: String = row.get(0)?;
                Ok((task_id, read_parts(row)?))
            })
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading episodes"))?;
        rows.into_iter()
            .map(|(task_id, parts)| {
                let task_id =
                    cortex_core::Ulid::parse(&task_id).map_err(|_| StoreError::NotText {
                        column: "episodes.task_id",
                    })?;
                let (localize, change, verify, pitfalls, outcome, created_at_ms) = parts;
                checked(
                    task_id,
                    [localize, change, verify, pitfalls],
                    (outcome, created_at_ms),
                )
            })
            .collect()
    }

    /// The record of the task `task_id`, when it has one.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn episode_of_task(
        self,
        task_id: cortex_core::Ulid,
    ) -> Result<Option<EpisodeRecord>, StoreError> {
        let raw = self
            .connection()
            .prepare_cached(schema::EPISODE_OF_TASK)
            .and_then(|mut statement| {
                statement
                    .query_row([task_id.to_string()], |row| read(row, task_id))
                    .optional()
            })
            .map_err(sqlite("reading an episode"))?;
        raw.transpose()
    }
}

/// `value` as the JSON it is stored as.
fn json<T: Serialize>(value: &T, column: &'static str) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::NotText { column })
}

/// The parts of one row, by the column order of the queries that read a record.
fn read_parts(row: &Row<'_>) -> rusqlite::Result<(String, String, String, String, String, i64)> {
    Ok((
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

/// One row as the record it holds.
fn read(
    row: &Row<'_>,
    task_id: cortex_core::Ulid,
) -> rusqlite::Result<Result<EpisodeRecord, StoreError>> {
    let (localize, change, verify, pitfalls, outcome, created_at_ms): (
        String,
        String,
        String,
        String,
        String,
        i64,
    ) = (
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    );
    Ok(checked(
        task_id,
        [localize, change, verify, pitfalls],
        (outcome, created_at_ms),
    ))
}

/// The record these stored values make, or the error that one of them cannot be read back.
fn checked(
    task_id: cortex_core::Ulid,
    [localize, change, verify, pitfalls]: [String; 4],
    (outcome, created_at_ms): (String, i64),
) -> Result<EpisodeRecord, StoreError> {
    Ok(EpisodeRecord {
        task_id,
        localize: of_json(&localize, "episodes.localize")?,
        change: of_json(&change, "episodes.change")?,
        verify: of_json(&verify, "episodes.verify")?,
        pitfalls: of_json(&pitfalls, "episodes.pitfalls")?,
        outcome: TaskOutcome::from_name(&outcome).ok_or(StoreError::NotText {
            column: "episodes.outcome",
        })?,
        created_at_ms,
    })
}

/// The value `text` holds as JSON.
fn of_json<T: for<'a> Deserialize<'a>>(text: &str, column: &'static str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|_| StoreError::NotText { column })
}
