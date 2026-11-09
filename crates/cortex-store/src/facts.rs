//! Facts: short project rules anchored to the files or symbols they are about. The store keeps each
//! fact's text, kind, source, status and use count, and each anchor's hash when the fact was made,
//! so that a later reader can tell when the code it is about changed. A corrected fact is
//! superseded by the one that corrects it: the old one stays, hidden from every reader of current
//! facts. Which status a fact moves to, and when, is decided by the learner; the store keeps what
//! it is told and refuses what it can never hold (text too long, no anchor).

use cortex_core::{ContentHash, FactKind, FactSource, FactStatus, MAX_FACT_CHARS};
use rusqlite::{Connection, OptionalExtension};

use crate::error::{StoreError, sqlite};
use crate::ids::{FactId, NodeId};
use crate::queries::{Queries, json_array};
use crate::{schema, values};

/// One anchor of a fact: the node it is about and that node's hash when the fact was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactAnchor {
    /// The file or symbol.
    pub node: NodeId,
    /// Its hash then.
    pub hash: ContentHash,
}

/// A fact as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRecord {
    /// The rule, at most [`MAX_FACT_CHARS`] characters.
    pub text: String,
    /// What kind of rule it is.
    pub kind: FactKind,
    /// Who wrote it.
    pub source: FactSource,
    /// Where it stands.
    pub status: FactStatus,
    /// When it was written, in UTC milliseconds.
    pub created_at_ms: i64,
    /// What it is about: at least one anchor, in node order.
    pub anchors: Vec<FactAnchor>,
}

/// A fact as the store holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRow {
    /// Its row.
    pub id: FactId,
    /// What was written, with its status now.
    pub record: FactRecord,
    /// How many times it was injected.
    pub uses: u64,
    /// When it was last confirmed true, in UTC milliseconds.
    pub last_verified_at_ms: i64,
    /// The fact that corrects it, if one does.
    pub superseded_by: Option<FactId>,
}

/// Adds `record` and its anchors, and returns its id.
///
/// # Errors
///
/// [`StoreError::FactTooLong`] for text over [`MAX_FACT_CHARS`] characters,
/// [`StoreError::FactUnanchored`] for no anchor, or SQLite errors.
pub(crate) fn add(connection: &Connection, record: &FactRecord) -> Result<FactId, StoreError> {
    let chars = record.text.chars().count();
    if chars > MAX_FACT_CHARS {
        return Err(StoreError::FactTooLong { chars });
    }
    if record.anchors.is_empty() {
        return Err(StoreError::FactUnanchored);
    }
    let id = connection
        .prepare_cached(schema::FACT_ADD)
        .and_then(|mut statement| {
            statement.insert((
                record.text.as_str(),
                record.kind.name(),
                record.source.name(),
                record.status.name(),
                record.created_at_ms,
            ))
        })
        .map_err(sqlite("adding a fact"))?;
    let mut anchor = connection
        .prepare_cached(schema::FACT_ANCHOR_ADD)
        .map_err(sqlite("anchoring a fact"))?;
    for each in &record.anchors {
        anchor
            .execute((id, each.node.get(), each.hash.as_bytes().as_slice()))
            .map_err(sqlite("anchoring a fact"))?;
    }
    Ok(FactId(id))
}

/// Sets fact `id`'s status to `status` at `at_ms`; a fact made active counts as confirmed then.
///
/// # Errors
///
/// [`StoreError::UnknownRow`] when no fact has `id`, or SQLite errors.
pub(crate) fn set_status(
    connection: &Connection,
    id: FactId,
    (status, at_ms): (FactStatus, i64),
) -> Result<(), StoreError> {
    let changed = connection
        .prepare_cached(schema::FACT_SET_STATUS)
        .and_then(|mut statement| {
            statement.execute((id.get(), status.name(), at_ms, status == FactStatus::Active))
        })
        .map_err(sqlite("setting a fact's status"))?;
    if changed == 0 {
        return Err(unknown(id));
    }
    Ok(())
}

/// Marks fact `old` superseded by fact `new`, which corrects it. Whether it was current.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn supersede(
    connection: &Connection,
    old: FactId,
    new: FactId,
) -> Result<bool, StoreError> {
    let changed = connection
        .prepare_cached(schema::FACT_SUPERSEDE)
        .and_then(|mut statement| statement.execute((old.get(), new.get())))
        .map_err(sqlite("superseding a fact"))?;
    Ok(changed == 1)
}

/// Counts one more injection of each of `ids`, each once however often it is listed.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn count_uses(connection: &Connection, ids: &[FactId]) -> Result<(), StoreError> {
    let listed = json_array(ids.iter().map(|id| id.get().to_string()));
    connection
        .prepare_cached(schema::FACT_COUNT_USES)
        .and_then(|mut statement| statement.execute([listed]))
        .map_err(sqlite("counting facts' uses"))?;
    Ok(())
}

/// The error for a fact `id` that does not exist.
fn unknown(id: FactId) -> StoreError {
    StoreError::UnknownRow {
        table: "facts",
        id: id.get(),
    }
}

/// A fact's columns as read: id, text, kind, source, status, uses, created, last verified, the
/// fact superseding it.
type Columns = (
    i64,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    Option<i64>,
);

impl Queries<'_> {
    /// Fact `id`, superseded or not; none when there is no such fact.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a row that is not what the store writes.
    pub fn fact(self, id: FactId) -> Result<Option<FactRow>, StoreError> {
        let columns = self
            .connection()
            .prepare_cached(schema::FACT_BY_ID)
            .and_then(|mut statement| statement.query_row([id.get()], columns_of).optional())
            .map_err(sqlite("reading a fact"))?;
        columns.map(|columns| self.row_of(columns)).transpose()
    }

    /// The current facts, those no other fact supersedes, in `status`, oldest first.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a row that is not what the store writes.
    pub fn current_facts(self, status: FactStatus) -> Result<Vec<FactRow>, StoreError> {
        let rows: Vec<Columns> = self
            .connection()
            .prepare_cached(schema::FACTS_CURRENT_WITH_STATUS)
            .and_then(|mut statement| {
                statement
                    .query_map([status.name()], columns_of)?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("reading facts"))?;
        rows.into_iter()
            .map(|columns| self.row_of(columns))
            .collect()
    }

    /// The ids of the current facts anchored to any of `nodes`, each once, in id order.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn facts_anchored_to(self, nodes: &[NodeId]) -> Result<Vec<FactId>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        let ids: Vec<i64> = self
            .connection()
            .prepare_cached(schema::FACTS_ANCHORED_TO)
            .and_then(|mut statement| {
                statement
                    .query_map([listed], |row| row.get(0))?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("reading the facts of nodes"))?;
        Ok(ids.into_iter().map(FactId).collect())
    }

    /// The fact `columns` hold, with its anchors read.
    fn row_of(self, columns: Columns) -> Result<FactRow, StoreError> {
        let (id, text, kind, source, status, uses, created, verified, by) = columns;
        let anchors: Vec<(i64, Vec<u8>)> = self
            .connection()
            .prepare_cached(schema::FACT_ANCHORS_OF)
            .and_then(|mut statement| {
                statement
                    .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("reading a fact's anchors"))?;
        let corrupt = |column: &'static str| StoreError::Corrupt { column, row: id };
        let anchors = anchors
            .into_iter()
            .map(|(node, hash)| {
                let hash: [u8; 32] = hash
                    .try_into()
                    .map_err(|_| corrupt("fact_anchors.anchor_hash"))?;
                Ok(FactAnchor {
                    node: NodeId(node),
                    hash: ContentHash::from_bytes(hash),
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(FactRow {
            id: FactId(id),
            record: FactRecord {
                text,
                kind: values::kind(&kind, FactKind::from_name, "facts.kind", id)?,
                source: values::kind(&source, FactSource::from_name, "facts.source", id)?,
                status: values::kind(&status, FactStatus::from_name, "facts.status", id)?,
                created_at_ms: created,
                anchors,
            },
            uses: u64::try_from(uses).map_err(|_| corrupt("facts.uses"))?,
            last_verified_at_ms: verified,
            superseded_by: by.map(FactId),
        })
    }
}

/// A fact's columns from `row`, in the order the fact queries select them.
fn columns_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<Columns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}
