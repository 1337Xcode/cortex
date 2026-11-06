//! The warnings Cortex gave after edits: one row for each file a warning named, with what the
//! warning was about, labelled resolved or not once its task ends.
//!
//! A row holds node ids, the kind of warning and its strength, never a file's name or code.

use cortex_core::{Ulid, WarningKind};
use rusqlite::{Connection, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::{NodeId, WarningId};
use crate::queries::Queries;
use crate::schema;

/// One file a warning named, as it is recorded with what the warning was about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarningRecord {
    /// The task it was given in.
    pub task_id: Ulid,
    /// When, in UTC milliseconds.
    pub at_ms: i64,
    /// What it points out.
    pub kind: WarningKind,
    /// What it is about: the edited file, or the symbol whose signature changed.
    pub source: NodeId,
    /// The file it named: the companion, the test, or a file that calls the changed symbol.
    pub target: NodeId,
    /// How strong its evidence was, from 0 to 1.
    pub strength: f64,
}

/// One warning as the store holds it: its record, and what the end of its task found of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarningRow {
    /// Its row.
    pub id: WarningId,
    /// What was recorded.
    pub record: WarningRecord,
    /// Whether its target was read or edited after it, once its task ended.
    pub resolved: Option<bool>,
}

/// Records `record`, returning its row.
pub(crate) fn record(
    connection: &Connection,
    record: &WarningRecord,
) -> Result<WarningId, StoreError> {
    let id = connection
        .prepare_cached(schema::WARNING_INSERT)
        .and_then(|mut statement| {
            statement.insert((
                record.task_id.to_string(),
                record.at_ms,
                record.kind.name(),
                record.source.get(),
                record.target.get(),
                record.strength,
            ))
        })
        .map_err(sqlite("recording a warning"))?;
    Ok(WarningId(id))
}

/// Sets whether warning `id` was resolved. Whether the store holds the warning.
pub(crate) fn resolve(
    connection: &Connection,
    id: WarningId,
    is_resolved: bool,
) -> Result<bool, StoreError> {
    let changed = connection
        .prepare_cached(schema::WARNING_RESOLVE)
        .and_then(|mut statement| statement.execute((id.get(), is_resolved)))
        .map_err(sqlite("resolving a warning"))?;
    Ok(changed == 1)
}

impl Queries<'_> {
    /// The warnings of task `task_id`, in the order they were recorded.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn warnings_of_task(self, task_id: Ulid) -> Result<Vec<WarningRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::WARNINGS_OF_TASK)
            .map_err(sqlite("reading warnings"))?;
        let rows = statement
            .query_map([task_id.to_string()], read)
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading warnings"))?;
        rows.into_iter().map(RawWarning::check).collect()
    }
}

/// A warning row as SQLite gives it, before its values are checked.
struct RawWarning {
    /// `warning_id`.
    id: i64,
    /// `task_id`.
    task_id: String,
    /// `ts`.
    at_ms: i64,
    /// `kind`.
    kind: String,
    /// `src_node` and `dst_node`.
    nodes: (i64, i64),
    /// `strength`.
    strength: f64,
    /// `resolved`, 0 or 1 when set.
    resolved: Option<i64>,
}

/// One row of a warnings query.
fn read(row: &Row<'_>) -> rusqlite::Result<RawWarning> {
    Ok(RawWarning {
        id: row.get(0)?,
        task_id: row.get(1)?,
        at_ms: row.get(2)?,
        kind: row.get(3)?,
        nodes: (row.get(4)?, row.get(5)?),
        strength: row.get(6)?,
        resolved: row.get(7)?,
    })
}

impl RawWarning {
    /// The row, its values checked.
    fn check(self) -> Result<WarningRow, StoreError> {
        let corrupt = |column: &'static str| StoreError::Corrupt {
            column,
            row: self.id,
        };
        let task_id = Ulid::parse(&self.task_id).map_err(|_| corrupt("warnings.task_id"))?;
        let kind = WarningKind::from_name(&self.kind).ok_or_else(|| corrupt("warnings.kind"))?;
        let resolved = match self.resolved {
            None => None,
            Some(0) => Some(false),
            Some(1) => Some(true),
            Some(_) => return Err(corrupt("warnings.resolved")),
        };
        Ok(WarningRow {
            id: WarningId(self.id),
            record: WarningRecord {
                task_id,
                at_ms: self.at_ms,
                kind,
                source: NodeId(self.nodes.0),
                target: NodeId(self.nodes.1),
                strength: self.strength,
            },
            resolved,
        })
    }
}
