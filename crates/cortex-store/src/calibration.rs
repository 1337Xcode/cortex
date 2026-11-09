//! The confidence model the gate uses once it has one, kept in the `calibration` table, and what it
//! is fitted on: the labelled injections of prompts with the gate's features. The learner reads a
//! window of them and keeps a model; every retrieval reads the newest model.
//!
//! The coefficients are numbers only: what each one weighs is the learner's to say. The last time
//! a model was fitted, kept or not, is kept in the `meta` table, so that upkeep fits again only
//! when there is more to learn from.

use cortex_core::{GateFeatures, HookKind};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{StoreError, sqlite};
use crate::meta::MetaKey;
use crate::queries::Queries;
use crate::schema;

/// One model kept: its version (the newest is in use), its coefficients, how many labelled
/// injections it was fitted on, how well it ranked those it was not fitted on, and when.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationRecord {
    /// One more than the model kept before it; the first is 1.
    pub version: u32,
    /// The intercept first, then a weight for each feature, in the learner's order.
    pub coefficients: Vec<f64>,
    /// How many labelled injections it was fitted on.
    pub samples: u32,
    /// The area under the ROC curve of its probabilities on the labelled injections held out from
    /// its fitting, from 0 to 1.
    pub auc: f64,
    /// When it was fitted, in UTC milliseconds.
    pub trained_at_ms: i64,
}

/// One labelled injection of a prompt, as a model is fitted on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelledInjection {
    /// The injection's id: later injections have larger ids.
    pub id: i64,
    /// What the gate weighed.
    pub features: GateFeatures,
    /// Whether its task used one of its first files.
    pub label: bool,
}

/// The last time a model was fitted, kept or not: how many labelled injections there were then,
/// and when, in UTC milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CalibrationAttempt {
    /// How many labelled injections of prompts the store held.
    pub labelled: u64,
    /// When, in UTC milliseconds.
    pub at_ms: i64,
}

impl Queries<'_> {
    /// The newest model kept, the one in use; none before the first is kept.
    ///
    /// # Errors
    ///
    /// SQLite errors, and [`StoreError::Corrupt`] for coefficients that are not a list of numbers
    /// or counts out of range.
    pub fn calibration(self) -> Result<Option<CalibrationRecord>, StoreError> {
        let raw: Option<(i64, String, i64, f64, i64)> = self
            .connection()
            .query_row(schema::CALIBRATION_LATEST, [], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .optional()
            .map_err(sqlite("reading the confidence model"))?;
        let Some((version, coefficients, samples, auc, trained_at_ms)) = raw else {
            return Ok(None);
        };
        let corrupt = |column| StoreError::Corrupt {
            column,
            row: version,
        };
        Ok(Some(CalibrationRecord {
            version: u32::try_from(version).map_err(|_| corrupt("calibration.version"))?,
            coefficients: serde_json::from_str(&coefficients)
                .map_err(|_| corrupt("calibration.coefficients"))?,
            samples: u32::try_from(samples).map_err(|_| corrupt("calibration.samples"))?,
            auc,
            trained_at_ms,
        }))
    }

    /// The latest `limit` labelled injections of prompts that carry the gate's features, the
    /// oldest of them first. Those the gate kept back and those of held-out tasks are among them:
    /// they are labelled from what the agent did without them.
    ///
    /// # Errors
    ///
    /// SQLite errors, and [`StoreError::Corrupt`] for features that cannot be read back.
    pub fn labelled_injections(self, limit: usize) -> Result<Vec<LabelledInjection>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let raw: Vec<(i64, String, bool)> = self
            .connection()
            .prepare_cached(schema::LABELLED_INJECTIONS)
            .and_then(|mut statement| {
                statement
                    .query_map((HookKind::Prompt.name(), limit), |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                    .collect::<rusqlite::Result<_>>()
            })
            .map_err(sqlite("reading the labelled injections"))?;
        raw.into_iter()
            .map(|(id, features, label)| {
                let features =
                    serde_json::from_str(&features).map_err(|_| StoreError::Corrupt {
                        column: "injections.features",
                        row: id,
                    })?;
                Ok(LabelledInjection {
                    id,
                    features,
                    label,
                })
            })
            .collect()
    }

    /// How many labelled injections of prompts that carry the gate's features the store holds.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn labelled_injection_count(self) -> Result<u64, StoreError> {
        let count: i64 = self
            .connection()
            .query_row(
                schema::COUNT_LABELLED_INJECTIONS,
                [HookKind::Prompt.name()],
                |row| row.get(0),
            )
            .map_err(sqlite("counting the labelled injections"))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// The last time a model was fitted; none before the first time.
    ///
    /// # Errors
    ///
    /// SQLite errors, and [`StoreError::CorruptMeta`] for a stored value that cannot be read back.
    pub fn calibration_attempt(self) -> Result<Option<CalibrationAttempt>, StoreError> {
        let key = MetaKey::CalibrationAttempt.name();
        let stored: Option<String> = self
            .connection()
            .query_row(schema::META_GET, [key], |row| row.get(0))
            .optional()
            .map_err(sqlite("reading the last fitting of the confidence model"))?;
        stored
            .map(|text| serde_json::from_str(&text).map_err(|_| StoreError::CorruptMeta { key }))
            .transpose()
    }
}

/// Keeps a model with `coefficients`, fitted on `samples` labelled injections at `trained_at_ms`,
/// whose held-out AUC is `auc`, as the newest, and deletes all but the newest `keep_versions`:
/// its version.
pub(crate) fn add(
    connection: &Connection,
    (coefficients, samples, auc, trained_at_ms): (&[f64], u32, f64, i64),
    keep_versions: u32,
) -> Result<u32, StoreError> {
    let text = serde_json::to_string(coefficients).map_err(|_| StoreError::NotText {
        column: "calibration.coefficients",
    })?;
    connection
        .execute(schema::CALIBRATION_ADD, (text, samples, auc, trained_at_ms))
        .map_err(sqlite("keeping a confidence model"))?;
    let version = connection.last_insert_rowid();
    connection
        .execute(schema::CALIBRATION_PRUNE, [keep_versions])
        .map_err(sqlite("deleting old confidence models"))?;
    u32::try_from(version).map_err(|_| StoreError::Corrupt {
        column: "calibration.version",
        row: version,
    })
}

/// Keeps `attempt` as the last time a model was fitted.
pub(crate) fn set_attempt(
    connection: &Connection,
    attempt: &CalibrationAttempt,
) -> Result<(), StoreError> {
    let text = serde_json::to_string(attempt).map_err(|_| StoreError::NotText {
        column: "meta.value",
    })?;
    connection
        .execute(schema::META_SET, (MetaKey::CalibrationAttempt.name(), text))
        .map(drop)
        .map_err(sqlite("writing the last fitting of the confidence model"))
}
