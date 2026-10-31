//! Why a store operation failed.
//!
//! Errors name the operation and the file, row or setting involved. They never carry stored text
//! other than repository paths.

use std::io;
use std::path::PathBuf;

use cortex_core::RepoPath;
use thiserror::Error;

/// Why a store operation failed.
#[derive(Debug, Error)]
pub enum StoreError {
    /// A file or folder of the store could not be created, opened or locked.
    #[error("{operation} {}: {source}", .path.display())]
    Io {
        /// What the store was doing.
        operation: &'static str,
        /// The file or folder.
        path: PathBuf,
        /// The cause.
        #[source]
        source: io::Error,
    },
    /// SQLite reported an error.
    #[error("{operation}: {source}")]
    Sqlite {
        /// What the store was doing.
        operation: &'static str,
        /// The cause.
        #[source]
        source: rusqlite::Error,
    },
    /// Another writer holds the lock on this database.
    #[error("another writer holds {}", .lock.display())]
    WriterBusy {
        /// The lock file.
        lock: PathBuf,
    },
    /// A connection setting did not take the required value.
    #[error("SQLite setting {name} is {found}, not {expected}")]
    Setting {
        /// The pragma.
        name: &'static str,
        /// The value Cortex requires.
        expected: String,
        /// The value SQLite reported after it was set.
        found: String,
    },
    /// The database file has tables but was not written by Cortex.
    #[error("the database has tables but no schema version written by Cortex")]
    UnknownSchema,
    /// The database was written by a newer Cortex.
    #[error(
        "the database schema is version {found}; this build supports up to version {supported}"
    )]
    NewerSchema {
        /// The version in the database.
        found: u32,
        /// The newest version this build knows.
        supported: u32,
    },
    /// A reader found a schema the writer has not brought up to date yet.
    #[error("the database schema is version {found}, not {expected}; the writer updates it")]
    SchemaNotCurrent {
        /// The version in the database.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },
    /// A migration failed; the database is left as it was before migrating.
    #[error("migration {version} failed: {source}")]
    Migration {
        /// The migration's number.
        version: u32,
        /// The cause.
        #[source]
        source: rusqlite::Error,
    },
    /// No database exists yet.
    #[error("no database at {}", .path.display())]
    Missing {
        /// Where the database was expected.
        path: PathBuf,
    },
    /// SQLite rolled the whole transaction back after an earlier error, so nothing in it can be
    /// written or committed any more.
    #[error("the transaction was rolled back after an earlier error; start a new one")]
    TransactionLost,
    /// A record points at a symbol by a position it cannot have.
    #[error("{path}: {what} {index} points at symbol {position}, which does not come before it")]
    Position {
        /// The file the record belongs to.
        path: RepoPath,
        /// `symbol` or `reference`.
        what: &'static str,
        /// The record's position in its list.
        index: usize,
        /// The position it points at.
        position: usize,
    },
    /// A number is too large for a database integer.
    #[error("{column} value {value} is too large to store")]
    TooLarge {
        /// The column.
        column: &'static str,
        /// The value.
        value: u64,
    },
    /// A stored value cannot be read back as the type its column holds.
    #[error("stored {column} of row {row} cannot be read")]
    Corrupt {
        /// The table and column, such as `symbols.kind`.
        column: &'static str,
        /// The row's id.
        row: i64,
    },
    /// A value to store cannot be written as the text its column holds: a path that is not
    /// UTF-8.
    #[error("{column} cannot hold a value that is not UTF-8 text")]
    NotText {
        /// The table and column, such as `sessions.cwd`.
        column: &'static str,
    },
    /// An update named a row that does not exist.
    #[error("no row {id} in {table}")]
    UnknownRow {
        /// The table.
        table: &'static str,
        /// The id.
        id: i64,
    },
}

/// Wraps a SQLite error with the operation that failed, for use with `map_err`.
pub(crate) fn sqlite(operation: &'static str) -> impl FnOnce(rusqlite::Error) -> StoreError {
    move |source| StoreError::Sqlite { operation, source }
}

/// Wraps an I/O error with the operation and path, for use with `map_err`.
pub(crate) fn io(
    operation: &'static str,
    path: &std::path::Path,
) -> impl FnOnce(io::Error) -> StoreError {
    let path = path.to_owned();
    move |source| StoreError::Io {
        operation,
        path,
        source,
    }
}
