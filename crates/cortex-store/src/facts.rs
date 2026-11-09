//! Facts: short project rules anchored to the files or symbols they are about. The store keeps each
//! fact's text, kind, source, status (and since when), the task it was first asserted in and its
//! use count, and each anchor's hash when the fact was made, so that a later reader can tell when
//! the code it is about changed: [`Queries::facts_with_changed_anchors`] compares them with the
//! hash an anchor would keep now. A corrected fact is superseded by the one that corrects it: the
//! old one stays, hidden from every reader of current facts. Which status a fact moves to, and
//! when, is decided by the learner; the store keeps what it is told and refuses what it can never
//! hold (text too long, no anchor).

use std::collections::BTreeMap;

use cortex_core::{FactKind, FactSource, FactStatus, MAX_FACT_CHARS, ShortHash, Ulid};
use rusqlite::{Connection, OptionalExtension};

use crate::error::{StoreError, sqlite};
use crate::ids::{FactId, NodeId};
use crate::queries::{Queries, json_array};
use crate::{schema, values};

/// One anchor of a fact: the node it is about and that node's hash when the fact was made or last
/// confirmed: the first 16 bytes of a file's content hash, a symbol's body hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactAnchor {
    /// The file or symbol.
    pub node: NodeId,
    /// Its hash then.
    pub hash: ShortHash,
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
    /// The task it was first asserted in: none for the user's and for imports.
    pub task: Option<Ulid>,
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
    /// When its status was last set, in UTC milliseconds: its making when it never changed.
    pub status_at_ms: i64,
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
                record.task.map(|task| task.to_string()),
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

/// Gives each of fact `id`'s `anchors` the hash it holds there, as when the fact is confirmed
/// against the code as it is now.
///
/// # Errors
///
/// [`StoreError::UnknownRow`] for a node the fact is not anchored to, or SQLite errors.
pub(crate) fn restamp(
    connection: &Connection,
    id: FactId,
    anchors: &[FactAnchor],
) -> Result<(), StoreError> {
    let mut restamp = connection
        .prepare_cached(schema::FACT_ANCHOR_RESTAMP)
        .map_err(sqlite("restamping a fact's anchors"))?;
    for anchor in anchors {
        let changed = restamp
            .execute((
                id.get(),
                anchor.node.get(),
                anchor.hash.as_bytes().as_slice(),
            ))
            .map_err(sqlite("restamping a fact's anchors"))?;
        if changed == 0 {
            return Err(StoreError::UnknownRow {
                table: "fact_anchors",
                id: anchor.node.get(),
            });
        }
    }
    Ok(())
}

/// Records `task` as the one that first asserted fact `id`, unless one already is. Whether it
/// was recorded.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn note_task(
    connection: &Connection,
    id: FactId,
    task: Ulid,
) -> Result<bool, StoreError> {
    let changed = connection
        .prepare_cached(schema::FACT_NOTE_TASK)
        .and_then(|mut statement| statement.execute((id.get(), task.to_string())))
        .map_err(sqlite("noting a fact's task"))?;
    Ok(changed == 1)
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

/// A fact's columns as read: id, text, kind, source, status, when it was set, the task first
/// asserting it, uses, created, last verified, the fact superseding it.
type Columns = (
    i64,
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
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

    /// The fact whose id is `number`, as a person typed it; none when no fact has it.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn fact_id(self, number: i64) -> Result<Option<FactId>, StoreError> {
        Ok(self
            .fact_ids(schema::FACT_ID_OF, [number])?
            .into_iter()
            .next())
    }

    /// Each of `nodes` that is a file or a symbol, named as a note names an anchor: the file's
    /// path, and for a symbol `:` and its qualified name. A node that is gone, or neither, is left
    /// out.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn anchor_names(self, nodes: &[NodeId]) -> Result<BTreeMap<NodeId, String>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        let rows: Vec<(i64, String, Option<String>)> = self
            .connection()
            .prepare_cached(schema::ANCHOR_NAMES_OF)
            .and_then(|mut statement| {
                statement
                    .query_map([listed], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("naming anchors"))?;
        Ok(rows
            .into_iter()
            .map(|(node, path, symbol)| {
                let name = match symbol {
                    Some(symbol) => format!("{path}:{symbol}"),
                    None => path,
                };
                (NodeId(node), name)
            })
            .collect())
    }

    /// The ids of the current candidate and active facts with an anchor whose node's hash is not
    /// the one kept, or that has none now (the node is gone, or its contents are not read), in
    /// id order.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn facts_with_changed_anchors(self) -> Result<Vec<FactId>, StoreError> {
        let statuses = (FactStatus::Candidate.name(), FactStatus::Active.name());
        self.fact_ids(schema::FACTS_WITH_CHANGED_ANCHORS, statuses)
    }

    /// The ids of the current facts in `status` since `at_ms` or before, in id order.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn facts_with_status_since(
        self,
        status: FactStatus,
        at_ms: i64,
    ) -> Result<Vec<FactId>, StoreError> {
        self.fact_ids(schema::FACTS_WITH_STATUS_SINCE, (status.name(), at_ms))
    }

    /// The hash an anchor on each of `nodes` would keep now, in node order: the first 16 bytes of
    /// a file's content hash, a symbol's body hash. A node with none (a folder, a file whose
    /// contents are not read, a node that does not exist) is left out.
    ///
    /// # Errors
    ///
    /// SQLite errors, or a hash that is not what the store writes.
    pub fn anchor_hashes(self, nodes: &[NodeId]) -> Result<Vec<FactAnchor>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        let rows: Vec<(i64, Vec<u8>)> = self
            .connection()
            .prepare_cached(schema::ANCHOR_HASHES_OF)
            .and_then(|mut statement| {
                statement
                    .query_map([listed], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("reading anchor hashes"))?;
        rows.into_iter()
            .map(|(node, hash)| anchor(node, hash, "anchor_hashes.hash", node))
            .collect()
    }

    /// The fact ids `sql` selects with `parameters`.
    fn fact_ids(
        self,
        sql: &str,
        parameters: impl rusqlite::Params,
    ) -> Result<Vec<FactId>, StoreError> {
        let ids: Vec<i64> = self
            .connection()
            .prepare_cached(sql)
            .and_then(|mut statement| {
                statement
                    .query_map(parameters, |row| row.get(0))?
                    .collect::<Result<_, _>>()
            })
            .map_err(sqlite("reading facts"))?;
        Ok(ids.into_iter().map(FactId).collect())
    }

    /// The ids of the current facts anchored to any of `nodes`, each once, in id order.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn facts_anchored_to(self, nodes: &[NodeId]) -> Result<Vec<FactId>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        self.fact_ids(schema::FACTS_ANCHORED_TO, [listed])
    }

    /// The fact `columns` hold, with its anchors read.
    fn row_of(self, columns: Columns) -> Result<FactRow, StoreError> {
        let (id, text, kind, source, status, status_at, task, uses, created, verified, by) =
            columns;
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
            .map(|(node, hash)| anchor(node, hash, "fact_anchors.anchor_hash", id))
            .collect::<Result<Vec<_>, StoreError>>()?;
        let task = task
            .map(|task| Ulid::parse(&task).map_err(|_| corrupt("facts.asserted_in")))
            .transpose()?;
        Ok(FactRow {
            id: FactId(id),
            record: FactRecord {
                text,
                kind: values::kind(&kind, FactKind::from_name, "facts.kind", id)?,
                source: values::kind(&source, FactSource::from_name, "facts.source", id)?,
                status: values::kind(&status, FactStatus::from_name, "facts.status", id)?,
                created_at_ms: created,
                task,
                anchors,
            },
            status_at_ms: status_at,
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
        row.get(9)?,
        row.get(10)?,
    ))
}

/// The anchor on `node` whose stored hash is `hash`, which must be 16 bytes; `column` of `row`
/// is corrupt otherwise.
fn anchor(
    node: i64,
    hash: Vec<u8>,
    column: &'static str,
    row: i64,
) -> Result<FactAnchor, StoreError> {
    let hash: [u8; 16] = hash
        .try_into()
        .map_err(|_| StoreError::Corrupt { column, row })?;
    Ok(FactAnchor {
        node: NodeId(node),
        hash: ShortHash::from_bytes(hash),
    })
}
