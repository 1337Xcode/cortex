//! The facts that need a person: those anchored to code the index no longer holds, and those
//! suspect since the code they are about changed. `cortex status` names them, and only when there
//! are some; `cortex facts` lists them to confirm or retire.

use cortex_core::FactStatus;

use crate::error::{StoreError, sqlite};
use crate::queries::Queries;
use crate::schema;

/// How many current facts need a person, each counted once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactHealth {
    /// Facts not retired with an anchor on a file or symbol the index no longer holds.
    pub gone: u64,
    /// Suspect facts whose anchors are all still held: their code changed since they were
    /// written or last confirmed.
    pub changed: u64,
}

impl Queries<'_> {
    /// How many current facts need a person: anchored to code that is gone, or suspect since
    /// their code changed.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn fact_health(self) -> Result<FactHealth, StoreError> {
        let (gone, changed): (i64, i64) = self
            .connection()
            .query_row(
                schema::FACT_HEALTH,
                (FactStatus::Retired.name(), FactStatus::Suspect.name()),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(sqlite("reading the facts that need a person"))?;
        Ok(FactHealth {
            gone: u64::try_from(gone).unwrap_or(0),
            changed: u64::try_from(changed).unwrap_or(0),
        })
    }
}
