//! What the self-check last found, kept in the `meta` table, and what it is worked out from: the
//! labels of the injections that reached the agent. The learner writes it as tasks end; every
//! retrieval reads it.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{StoreError, sqlite};
use crate::meta::MetaKey;
use crate::queries::Queries;
use crate::schema;

/// What the self-check last found. Before it first ran, everything is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SelfCheckRecord {
    /// How many steps the gate's bar stands raised.
    pub raise_steps: u32,
    /// How many delivered injections were labelled when the bar was last raised; 0 before any
    /// raise.
    pub raised_at: u64,
    /// How many of the latest delivered, labelled injections the recent precision covers.
    pub recent_labelled: u32,
    /// How many of those were labelled used.
    pub recent_used: u32,
}

impl Queries<'_> {
    /// What the self-check last found.
    ///
    /// # Errors
    ///
    /// SQLite errors, and [`StoreError::CorruptMeta`] for a stored value that cannot be read back.
    pub fn self_check(self) -> Result<SelfCheckRecord, StoreError> {
        let key = MetaKey::SelfCheck.name();
        let stored: Option<String> = self
            .connection()
            .query_row(schema::META_GET, [key], |row| row.get(0))
            .optional()
            .map_err(sqlite("reading the self-check"))?;
        match stored {
            None => Ok(SelfCheckRecord::default()),
            Some(text) => serde_json::from_str(&text).map_err(|_| StoreError::CorruptMeta { key }),
        }
    }

    /// The labels of the latest `limit` injections that reached the agent and are labelled, the
    /// newest first. An injection reached the agent when it was neither kept back by the gate nor
    /// held out and its text has a hash, as [`crate::InjectionRecord::was_delivered`] says.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn delivered_labels(self, limit: usize) -> Result<Vec<bool>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.connection()
            .prepare_cached(schema::DELIVERED_LABELS)
            .and_then(|mut statement| {
                statement
                    .query_map([limit], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<bool>>>()
            })
            .map_err(sqlite("reading the labels of delivered injections"))
    }

    /// How many injections that reached the agent are labelled.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn delivered_labelled(self) -> Result<u64, StoreError> {
        let count: i64 = self
            .connection()
            .query_row(schema::COUNT_DELIVERED_LABELLED, [], |row| row.get(0))
            .map_err(sqlite("counting the labelled delivered injections"))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// How many injections that reached the agent are labelled, and how many of them were used:
    /// the precision of everything Cortex gave that the end of its task judged.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn delivered_precision(self) -> Result<(u64, u64), StoreError> {
        let (labelled, used): (i64, i64) = self
            .connection()
            .query_row(schema::DELIVERED_PRECISION, [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(sqlite("counting the labelled delivered injections used"))?;
        Ok((
            u64::try_from(labelled).unwrap_or(0),
            u64::try_from(used).unwrap_or(0),
        ))
    }
}

/// Keeps `record` as what the self-check last found.
pub(crate) fn set(connection: &Connection, record: &SelfCheckRecord) -> Result<(), StoreError> {
    let text = serde_json::to_string(record).map_err(|_| StoreError::NotText {
        column: "meta.value",
    })?;
    connection
        .execute(schema::META_SET, (MetaKey::SelfCheck.name(), text))
        .map(drop)
        .map_err(sqlite("writing the self-check"))
}
