//! Clusters of tasks that changed much the same files, which a later task's prompt is matched
//! against to find what tasks like it changed.
//!
//! A cluster holds the words of its files' paths, the files themselves, how many tasks it covers
//! and when it was last written: node ids, counts, a time and words of paths, nothing of the work.
//! The clustering reads every record it is given and writes the clusters whole, so the rows are
//! replaced together rather than changed one at a time; whoever writes them keeps their lists
//! bounded.

use cortex_core::Identifier;
use rusqlite::{Connection, Row};
use serde::Serialize;

use crate::error::{StoreError, sqlite};
use crate::ids::NodeId;
use crate::queries::Queries;
use crate::schema;

/// A cluster of tasks that changed much the same files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchetypeRecord {
    /// The words of the paths of the files such tasks change, in order, which the words of a
    /// prompt are matched against.
    pub seed_terms: Vec<Identifier>,
    /// The files such tasks change, the ones most of them changed first.
    pub files: Vec<NodeId>,
    /// How many tasks the cluster covers.
    pub episode_count: u32,
    /// When the cluster was last written, in UTC milliseconds.
    pub updated_at_ms: i64,
}

/// Writes `archetypes`, replacing every cluster stored before.
pub(crate) fn replace(
    connection: &Connection,
    archetypes: &[ArchetypeRecord],
) -> Result<(), StoreError> {
    connection
        .execute(schema::ARCHETYPES_DELETE_ALL, [])
        .map_err(sqlite("deleting clusters"))?;
    let mut statement = connection
        .prepare_cached(schema::ARCHETYPE_INSERT)
        .map_err(sqlite("adding a cluster"))?;
    for archetype in archetypes {
        let files: Vec<i64> = archetype.files.iter().map(|node| node.get()).collect();
        statement
            .execute((
                json(&archetype.seed_terms, "archetypes.seed_terms")?,
                json(&files, "archetypes.files")?,
                archetype.episode_count,
                archetype.updated_at_ms,
            ))
            .map_err(sqlite("adding a cluster"))?;
    }
    Ok(())
}

impl Queries<'_> {
    /// Every cluster, oldest first.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn archetypes(self) -> Result<Vec<ArchetypeRecord>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::ARCHETYPES)
            .map_err(sqlite("reading clusters"))?;
        let rows = statement
            .query_map([], read)
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading clusters"))?;
        rows.into_iter().collect()
    }
}

/// `value` as the JSON it is stored as.
fn json<T: Serialize>(value: &T, column: &'static str) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::NotText { column })
}

/// One row as the cluster it holds.
fn read(row: &Row<'_>) -> rusqlite::Result<Result<ArchetypeRecord, StoreError>> {
    let (seed_terms, files, episode_count, updated_at_ms): (String, String, u32, i64) =
        (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
    Ok(checked(
        (&seed_terms, &files),
        (episode_count, updated_at_ms),
    ))
}

/// The cluster these stored values make, or the error that one of them cannot be read back.
fn checked(
    (seed_terms, files): (&str, &str),
    (episode_count, updated_at_ms): (u32, i64),
) -> Result<ArchetypeRecord, StoreError> {
    let numbers: Vec<i64> = serde_json::from_str(files).map_err(|_| StoreError::NotText {
        column: "archetypes.files",
    })?;
    Ok(ArchetypeRecord {
        seed_terms: serde_json::from_str(seed_terms).map_err(|_| StoreError::NotText {
            column: "archetypes.seed_terms",
        })?,
        files: numbers.into_iter().map(NodeId).collect(),
        episode_count,
        updated_at_ms,
    })
}
