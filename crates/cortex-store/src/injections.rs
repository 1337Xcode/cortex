//! What Cortex gave an agent, or would have given: one row for each primer given and for each
//! prompt's retrieval that reached the gate, whatever the gate said, labelled once its task ends.
//!
//! A row holds node ids, line ranges, the kinds of the reasons, counts, the gate's numbers and the
//! hash of the text, never a prompt's words, a file's name or code: what a reason said in words is
//! not kept, only what kind of reason it was.

use cortex_core::{ContentHash, GateFeatures, HookKind, Identifier, ReasonKind, Ulid};
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
    /// The hash of the text given, or that would have been.
    pub text_hash: Option<ContentHash>,
    /// The gate's numbers; none for a primer.
    pub features: Option<GateFeatures>,
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

impl Queries<'_> {
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
