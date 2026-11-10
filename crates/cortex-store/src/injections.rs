//! What Cortex gave an agent, or would have given: one row for each primer given and for each
//! prompt's retrieval that reached the gate, whatever the gate said, labelled once its task ends.
//!
//! A row holds node ids, line ranges, the kinds of the reasons, counts, the gate's numbers and the
//! hash of the text, never a prompt's words, a file's name or code: what a reason said in words is
//! not kept, only what kind of reason it was.

use cortex_core::{ContentHash, GateFeatures, HookKind, Identifier, ReasonKind, ShortHash, Ulid};
use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};

use crate::error::{StoreError, sqlite};
use crate::ids::{InjectionId, NodeId};
use crate::queries::Queries;
use crate::schema;

/// One file an injection gave, or weighed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectedFile {
    /// The file's node.
    #[serde(with = "as_number")]
    pub node: NodeId,
    /// The first and last lines of each of its spans the text gave, the strongest first.
    pub spans: Vec<(u32, u32)>,
    /// The kind of reason it was listed for; none for a file only weighed.
    pub reason: Option<ReasonKind>,
    /// The first and last lines of each of its spans whose code the text showed, from the file
    /// whose short hash is `content`; none from a record that kept no such spans.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shown: Vec<(u32, u32)>,
    /// The short hash of what the file held when the text was worked out, the file its spans and
    /// shown code are lines of; none when it was not known, and in records from before every
    /// file's was kept, none when no code was shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ShortHash>,
}

/// A node id as the number it is stored as.
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

/// One injection, as it is recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct InjectionRecord {
    /// The task the prompt began; none for a primer, which is a session's.
    pub task_id: Option<Ulid>,
    /// The session it came in.
    pub session_id: Identifier,
    /// The hook it answered: a session's start or a prompt.
    pub hook: HookKind,
    /// When, in UTC milliseconds.
    pub at_ms: i64,
    /// The files given, the best first; for a retrieval the gate kept back, the files it weighed.
    pub files: Vec<InjectedFile>,
    /// The tokens the text took, as estimated, or would have; 0 when there was no text.
    pub tokens: u32,
    /// The calibrated model's probability, once there is a model.
    pub confidence: Option<f64>,
    /// Whether the gate kept the text back.
    pub is_gated_out: bool,
    /// Whether the task was held out, so that nothing was given.
    pub is_holdout: bool,
    /// The hash of the text given, or that would have been for a held-out task; none when no
    /// text was given: the gate kept it back, or its answer could not be written.
    pub text_hash: Option<ContentHash>,
    /// The gate's numbers; none for a primer.
    pub features: Option<GateFeatures>,
}

impl InjectionRecord {
    /// Whether its text reached the agent: the gate let it through, the task was not held out,
    /// and the text was written in time, so that it has a hash.
    #[must_use]
    pub fn was_delivered(&self) -> bool {
        !self.is_gated_out && !self.is_holdout && self.text_hash.is_some()
    }
}

/// One injection as the store holds it: its record, and what its task's end said of it.
#[derive(Debug, Clone, PartialEq)]
pub struct InjectionRow {
    /// Its row.
    pub id: InjectionId,
    /// What was recorded.
    pub record: InjectionRecord,
    /// How many of its files the task later read or edited, once the task ended.
    pub used_count: Option<u32>,
    /// Whether one of its top three files was read and used, or edited, once the task ended.
    pub label: Option<bool>,
}

/// Records `record`, returning its row.
pub(crate) fn record(
    connection: &Connection,
    record: &InjectionRecord,
) -> Result<InjectionId, StoreError> {
    let files = serde_json::to_string(&record.files).map_err(|_| StoreError::NotText {
        column: "injections.items",
    })?;
    let features = record
        .features
        .map(|features| serde_json::to_string(&features))
        .transpose()
        .map_err(|_| StoreError::NotText {
            column: "injections.features",
        })?;
    let id = connection
        .prepare_cached(schema::INJECTION_INSERT)
        .and_then(|mut statement| {
            statement.insert((
                record.task_id.map(|task| task.to_string()),
                record.session_id.as_str(),
                record.hook.name(),
                record.at_ms,
                files,
                record.tokens,
                record.confidence,
                record.is_gated_out,
                record.is_holdout,
                record.text_hash.as_ref().map(|hash| *hash.as_bytes()),
                features,
            ))
        })
        .map_err(sqlite("recording an injection"))?;
    Ok(InjectionId(id))
}

/// Sets what the end of its task found of injection `id`: `used_count`, how many of its files the
/// task read or edited, and `label`, whether one of its first three was used. Whether the store
/// holds the injection.
pub(crate) fn label(
    connection: &Connection,
    id: InjectionId,
    (used_count, label): (u32, bool),
) -> Result<bool, StoreError> {
    let changed = connection
        .prepare_cached(schema::INJECTION_LABEL)
        .and_then(|mut statement| statement.execute((id.get(), used_count, label)))
        .map_err(sqlite("labelling an injection"))?;
    Ok(changed == 1)
}

/// Sets how many of injection `id`'s files its task read or edited, leaving its label as it is:
/// whether the injection was found.
pub(crate) fn count_use(
    connection: &Connection,
    id: InjectionId,
    used_count: u32,
) -> Result<bool, StoreError> {
    let changed = connection
        .prepare_cached(schema::INJECTION_USE)
        .and_then(|mut statement| statement.execute((id.get(), used_count)))
        .map_err(sqlite("counting an injection's use"))?;
    Ok(changed == 1)
}

impl Queries<'_> {
    /// How many lines naming the file earlier tasks read next reached the agent and had their
    /// task's end count their use, and how many of those named a file the task then read or
    /// edited.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn next_file_use(self) -> Result<(u64, u64), StoreError> {
        let (named, used): (i64, i64) = self
            .connection()
            .query_row(
                schema::NEXT_FILE_USE,
                [ReasonKind::ReadNext.name()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(sqlite("counting the use of next-file lines"))?;
        Ok((
            u64::try_from(named).unwrap_or(0),
            u64::try_from(used).unwrap_or(0),
        ))
    }

    /// Whether a text that reached task `task_id`'s agent named the file whose node is `node` for
    /// the reason `reason`, found by the index of the task's injections.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn has_named(
        self,
        task_id: Ulid,
        node: NodeId,
        reason: ReasonKind,
    ) -> Result<bool, StoreError> {
        self.connection()
            .prepare_cached(schema::TASK_NAMED)
            .and_then(|mut statement| {
                statement.query_row((task_id.to_string(), node.get(), reason.name()), |row| {
                    row.get(0)
                })
            })
            .map_err(sqlite("reading whether a task was given a file"))
    }

    /// The injections of task `task_id`, in the order they were recorded.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn injections_of_task(self, task_id: Ulid) -> Result<Vec<InjectionRow>, StoreError> {
        self.injections(schema::INJECTIONS_OF_TASK, &task_id.to_string())
    }

    /// The injections of session `session_id`, in the order they were recorded.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn injections_of_session(
        self,
        session_id: &Identifier,
    ) -> Result<Vec<InjectionRow>, StoreError> {
        self.injections(schema::INJECTIONS_OF_SESSION, session_id.as_str())
    }

    /// The work orders that reached session `session_id`'s agent after `after_ms`, the latest
    /// first, at most `limit` of them: prompts' injections, neither kept back nor held out, with
    /// the hash of a text written. They are found by the session's index of injections.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn work_orders_since(
        self,
        session_id: &Identifier,
        after_ms: i64,
        limit: usize,
    ) -> Result<Vec<InjectionRow>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut statement = self
            .connection()
            .prepare_cached(schema::WORK_ORDERS_SINCE)
            .map_err(sqlite("reading work orders"))?;
        let rows = statement
            .query_map((session_id.as_str(), after_ms, limit), read)
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading work orders"))?;
        rows.into_iter().map(RawInjection::check).collect()
    }

    /// The injections `sql` selects for `key`.
    fn injections(self, sql: &str, key: &str) -> Result<Vec<InjectionRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(sql)
            .map_err(sqlite("reading injections"))?;
        let rows = statement
            .query_map([key], read)
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading injections"))?;
        rows.into_iter().map(RawInjection::check).collect()
    }
}

/// An injection row as SQLite gives it, before its values are checked.
struct RawInjection {
    /// `injection_id`.
    id: i64,
    /// `task_id`.
    task_id: Option<String>,
    /// `session_id`.
    session_id: String,
    /// `hook`.
    hook: String,
    /// `ts`.
    at_ms: i64,
    /// `items`.
    files: String,
    /// `tokens_est`.
    tokens: i64,
    /// `confidence`.
    confidence: Option<f64>,
    /// `gated_out` and `holdout`.
    flags: (bool, bool),
    /// `rendered_hash`.
    text_hash: Option<[u8; 32]>,
    /// `used_count` and `label`.
    labelled: (Option<i64>, Option<bool>),
    /// `features`.
    features: Option<String>,
}

/// One row of an injections query.
fn read(row: &Row<'_>) -> rusqlite::Result<RawInjection> {
    Ok(RawInjection {
        id: row.get(0)?,
        task_id: row.get(1)?,
        session_id: row.get(2)?,
        hook: row.get(3)?,
        at_ms: row.get(4)?,
        files: row.get(5)?,
        tokens: row.get(6)?,
        confidence: row.get(7)?,
        flags: (row.get(8)?, row.get(9)?),
        text_hash: row.get(10)?,
        labelled: (row.get(11)?, row.get(12)?),
        features: row.get(13)?,
    })
}

impl RawInjection {
    /// The row, its values checked.
    fn check(self) -> Result<InjectionRow, StoreError> {
        let corrupt = |column: &'static str| StoreError::Corrupt {
            column,
            row: self.id,
        };
        let task_id = self
            .task_id
            .as_deref()
            .map(|text| Ulid::parse(text).map_err(|_| corrupt("injections.task_id")))
            .transpose()?;
        let session_id =
            Identifier::new(&self.session_id).map_err(|_| corrupt("injections.session_id"))?;
        let hook = HookKind::from_name(&self.hook).ok_or_else(|| corrupt("injections.hook"))?;
        let files: Vec<InjectedFile> =
            serde_json::from_str(&self.files).map_err(|_| corrupt("injections.items"))?;
        let tokens = u32::try_from(self.tokens).map_err(|_| corrupt("injections.tokens_est"))?;
        let features = self
            .features
            .as_deref()
            .map(|text| serde_json::from_str(text).map_err(|_| corrupt("injections.features")))
            .transpose()?;
        let used_count = self
            .labelled
            .0
            .map(|count| u32::try_from(count).map_err(|_| corrupt("injections.used_count")))
            .transpose()?;
        Ok(InjectionRow {
            id: InjectionId(self.id),
            record: InjectionRecord {
                task_id,
                session_id,
                hook,
                at_ms: self.at_ms,
                files,
                tokens,
                confidence: self.confidence,
                is_gated_out: self.flags.0,
                is_holdout: self.flags.1,
                text_hash: self.text_hash.map(ContentHash::from_bytes),
                features,
            },
            used_count,
            label: self.labelled.1,
        })
    }
}
