//! A read-only connection to a repository's database.

use std::path::Path;

use rusqlite::Connection;

use crate::connection::{self, Access};
use crate::error::{StoreError, io};
use crate::layout;
use crate::migrate;
use crate::queries::Queries;
use crate::schema::SCHEMA_VERSION;

/// A read-only connection. It never changes the database, so any number can be open beside the
/// writer.
#[derive(Debug)]
pub struct Reader {
    /// The read-only connection.
    connection: Connection,
}

impl Reader {
    /// Opens the database in the `.cortex` folder `folder` for reading.
    ///
    /// # Errors
    ///
    /// [`StoreError::Missing`] when there is no database yet, [`StoreError::SchemaNotCurrent`]
    /// when the writer has not brought it up to this build's schema, [`StoreError::NewerSchema`]
    /// when a newer Cortex wrote it, and I/O and SQLite errors.
    pub fn open(folder: &Path, mmap_bytes: u64) -> Result<Self, StoreError> {
        let database = layout::database(folder);
        let exists = database
            .try_exists()
            .map_err(io("looking for the database", &database))?;
        if !exists {
            return Err(StoreError::Missing { path: database });
        }
        // The schema is checked before the settings: a database the writer has not set up yet
        // also lacks its journal mode, and the schema says why.
        let connection = connection::connect(&database, Access::ReadOnly)?;
        let found = migrate::schema_version(&connection)?;
        if found > SCHEMA_VERSION {
            return Err(StoreError::NewerSchema {
                found,
                supported: SCHEMA_VERSION,
            });
        }
        if found < SCHEMA_VERSION {
            return Err(StoreError::SchemaNotCurrent {
                found,
                expected: SCHEMA_VERSION,
            });
        }
        connection::configure(&connection, Access::ReadOnly, mmap_bytes)?;
        Ok(Self { connection })
    }

    /// Read queries over the committed database.
    #[must_use]
    pub fn queries(&self) -> Queries<'_> {
        Queries::new(&self.connection)
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::RepoId;

    use super::*;
    use crate::writer::{Writer, WriterOptions};

    /// A reader's connection takes the same settings as the writer's: SQLite reports `synchronous`
    /// NORMAL as 1, `foreign_keys` ON as 1 and `temp_store` MEMORY as 2.
    #[test]
    fn readers_take_the_settings() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let folder = root.path().join(".cortex");
        let repo_id = RepoId::of(root.path());
        let _writer = Writer::open(&WriterOptions {
            folder: &folder,
            repo_id: &repo_id,
            mmap_bytes: 1 << 20,
            now_ms: 1,
        })?;
        let reader = Reader::open(&folder, 1 << 21)?;
        let integer = |name: &str| -> rusqlite::Result<i64> {
            reader
                .connection
                .pragma_query_value(None, name, |row| row.get(0))
        };
        let mode: String = reader
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        assert_eq!(mode, "wal");
        assert_eq!(integer("synchronous")?, 1);
        assert_eq!(integer("foreign_keys")?, 1);
        assert_eq!(integer("busy_timeout")?, 2000);
        assert_eq!(integer("mmap_size")?, 2_097_152);
        assert_eq!(integer("temp_store")?, 2);
        Ok(())
    }
}
