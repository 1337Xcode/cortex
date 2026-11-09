//! Opening SQLite connections with the settings Cortex requires, each read back after it is set.
//!
//! Write-ahead logging lets readers keep reading while the writer commits. With
//! `synchronous=NORMAL` a commit does not wait for the disk: a power cut can lose the last commits
//! but leaves the database intact. Temporary tables stay in memory. A connection that does not
//! take a setting is refused rather than used.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, ToSql};

use crate::error::{StoreError, sqlite};

/// How long a connection waits for another connection's lock before it fails: two seconds, so a
/// reader does not fail while the writer commits a batch.
const BUSY_TIMEOUT_MS: i64 = 2000;

/// How many prepared statements a connection keeps for reuse: more than the store has, so none
/// is prepared twice.
const STATEMENT_CACHE_CAPACITY: usize = 64;

/// How many pages the write-ahead log holds before a commit of the writer checkpoints it itself:
/// 64 MiB of 4 KiB pages. The checkpointer thread copies the log every second, so this checkpoint
/// copies at most about a second of writes. It is there because only a checkpoint that has copied
/// everything lets the next transaction start the log afresh, and a writer that commits without
/// pause never leaves the thread that moment: during a first index of a 7,700-file repository the
/// log grew to over 290 MB with no checkpoint of its own.
pub(crate) const LOG_RESTART_PAGES: i64 = 16_384;

/// How large the write-ahead log file stays once the log has started afresh, in bytes: SQLite
/// reuses the file rather than shrinking it, and without a limit it would keep the 64 MiB it grew
/// to. 4 MiB is what SQLite's own checkpoints let it reach.
pub(crate) const LOG_KEPT_BYTES: i64 = 4_194_304;

/// What a connection may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// The writer's connection: creates the database file when it is missing.
    ReadWrite,
    /// A reader's connection.
    ReadOnly,
}

/// Opens the database at `path` without applying any setting.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn connect(path: &Path, access: Access) -> Result<Connection, StoreError> {
    let flags = match access {
        Access::ReadWrite => {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
        }
        Access::ReadOnly => OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    };
    let connection =
        Connection::open_with_flags(path, flags).map_err(sqlite("opening the database"))?;
    connection.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);
    Ok(connection)
}

/// Applies the settings to `connection` and checks each one.
///
/// # Errors
///
/// SQLite errors, and [`StoreError::Setting`] when a setting reads back with another value.
pub(crate) fn configure(
    connection: &Connection,
    access: Access,
    mmap_bytes: u64,
) -> Result<(), StoreError> {
    if access == Access::ReadWrite {
        vacuum_incrementally_when_new(connection)?;
    }
    // The journal mode is stored in the database file, so only the writer sets it; a reader
    // checks that the writer did.
    let journal_mode: String = match access {
        Access::ReadWrite => {
            connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
        }
        Access::ReadOnly => connection.pragma_query_value(None, "journal_mode", |row| row.get(0)),
    }
    .map_err(sqlite("setting journal_mode"))?;
    check("journal_mode", "wal", &journal_mode)?;
    set(connection, "synchronous", "NORMAL", 1)?;
    set(connection, "foreign_keys", "ON", 1)?;
    set(connection, "busy_timeout", BUSY_TIMEOUT_MS, BUSY_TIMEOUT_MS)?;
    if access == Access::ReadWrite {
        // The writer's commits leave the log to its checkpointer until it holds this many pages. A
        // reader never commits, so it never checkpoints either way.
        set(
            connection,
            "wal_autocheckpoint",
            LOG_RESTART_PAGES,
            LOG_RESTART_PAGES,
        )?;
        set(
            connection,
            "journal_size_limit",
            LOG_KEPT_BYTES,
            LOG_KEPT_BYTES,
        )?;
    }
    let mmap_size = crate::values::integer(mmap_bytes, "mmap_size")?;
    set(connection, "mmap_size", mmap_size, mmap_size)?;
    set(connection, "temp_store", "MEMORY", 2)
}

/// Has a database that holds no table yet free its unused pages when asked, a few at a time
/// (`auto_vacuum` INCREMENTAL), so that upkeep can give back what it deleted without rewriting the
/// file. SQLite takes the setting only before the first table is created: a database made by an
/// earlier build keeps its setting.
fn vacuum_incrementally_when_new(connection: &Connection) -> Result<(), StoreError> {
    let tables: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .map_err(sqlite("reading whether the database is new"))?;
    if tables == 0 {
        set(connection, "auto_vacuum", "INCREMENTAL", 2)?;
    }
    Ok(())
}

/// Sets the pragma `name` to `value` and checks that it reads back as `expected`.
fn set(
    connection: &Connection,
    name: &'static str,
    value: impl ToSql,
    expected: i64,
) -> Result<(), StoreError> {
    connection
        .pragma_update(None, name, value)
        .map_err(sqlite("setting a pragma"))?;
    let found: i64 = connection
        .pragma_query_value(None, name, |row| row.get(0))
        .map_err(sqlite("reading a pragma"))?;
    check(name, &expected.to_string(), &found.to_string())
}

/// Refuses a setting that reads back as anything but `expected`.
fn check(name: &'static str, expected: &str, found: &str) -> Result<(), StoreError> {
    if found == expected {
        Ok(())
    } else {
        Err(StoreError::Setting {
            name,
            expected: expected.to_owned(),
            found: found.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default for `store.mmap_bytes`, 256 MiB, typed here rather than read from the
    /// configuration code.
    const MMAP_BYTES: u64 = 256 * 1024 * 1024;

    /// Opens the database at `path` with every setting applied.
    fn open(path: &Path, access: Access, mmap_bytes: u64) -> Result<Connection, StoreError> {
        let connection = connect(path, access)?;
        configure(&connection, access, mmap_bytes)?;
        Ok(connection)
    }

    /// Reads one pragma as text, whatever its type.
    fn pragma(connection: &Connection, name: &str) -> rusqlite::Result<String> {
        connection.pragma_query_value(None, name, |row| {
            Ok(match row.get_ref(0)? {
                rusqlite::types::ValueRef::Integer(number) => number.to_string(),
                rusqlite::types::ValueRef::Text(text) => String::from_utf8_lossy(text).into_owned(),
                other => format!("{other:?}"),
            })
        })
    }

    /// Every setting reads back as required, on the writer's connection and on a reader's.
    /// SQLite reports `synchronous` NORMAL as 1, `foreign_keys` ON as 1 and `temp_store` MEMORY
    /// as 2. The writer's commits checkpoint only past 16,384 pages; a reader keeps SQLite's 1,000
    /// pages, which it never reaches, since it never commits.
    #[test]
    fn settings_read_back() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("cortex.db");
        let expected = [
            ("journal_mode", "wal"),
            ("synchronous", "1"),
            ("foreign_keys", "1"),
            ("busy_timeout", "2000"),
            ("mmap_size", "268435456"),
            ("temp_store", "2"),
        ];
        let writer = open(&path, Access::ReadWrite, MMAP_BYTES)?;
        let reader = open(&path, Access::ReadOnly, MMAP_BYTES)?;
        for connection in [&writer, &reader] {
            for (name, value) in expected {
                assert_eq!(pragma(connection, name)?, value, "{name}");
            }
        }
        assert_eq!(pragma(&writer, "wal_autocheckpoint")?, "16384");
        assert_eq!(pragma(&writer, "journal_size_limit")?, "4194304");
        assert_eq!(pragma(&reader, "wal_autocheckpoint")?, "1000");
        Ok(())
    }

    /// A memory map larger than SQLite allows reads back smaller and is refused, naming the
    /// setting. SQLite caps the map at 2,147,418,112 bytes on Linux, macOS and Windows.
    #[test]
    fn a_setting_that_does_not_hold_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("cortex.db");
        let error = open(&path, Access::ReadWrite, 3_000_000_000).err();
        assert!(
            matches!(
                &error,
                Some(StoreError::Setting { name: "mmap_size", expected, found })
                    if expected == "3000000000" && found == "2147418112"
            ),
            "{error:?}"
        );
        Ok(())
    }
}
