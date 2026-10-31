//! Bringing a database's schema up to date.
//!
//! The database records the number of the last migration applied as `schema_version` in `meta`;
//! a new database has no tables and counts as version 0. Before migrating a database that already
//! has a schema, the store copies it to the backups folder, one copy per schema version, replacing
//! the copy an earlier attempt left. Then every pending migration runs in one transaction, so a
//! failure leaves the database exactly as it was.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, MAIN_DB, OptionalExtension, TransactionBehavior};

use crate::error::{StoreError, io, sqlite};
use crate::meta::MetaKey;
use crate::private;
use crate::schema::{self, Migration};

/// The database's schema version, when this build can migrate from it.
///
/// # Errors
///
/// [`StoreError::UnknownSchema`] for a database Cortex did not write, [`StoreError::NewerSchema`]
/// for one a newer Cortex wrote, and SQLite errors. Nothing is changed.
pub(crate) fn check(connection: &Connection, migrations: &[Migration]) -> Result<u32, StoreError> {
    let found = schema_version(connection)?;
    let latest = latest_version(migrations);
    if found > latest {
        return Err(StoreError::NewerSchema {
            found,
            supported: latest,
        });
    }
    Ok(found)
}

/// Applies the `migrations` the database has not had yet, after backing it up into `backups`.
/// Returns the backup's path, if one was made.
///
/// # Errors
///
/// The errors of [`check`], [`StoreError::Migration`] when a migration fails, and I/O and SQLite
/// errors.
pub(crate) fn migrate(
    connection: &mut Connection,
    migrations: &[Migration],
    backups: &Path,
) -> Result<Option<PathBuf>, StoreError> {
    let found = check(connection, migrations)?;
    let latest = latest_version(migrations);
    if found == latest {
        return Ok(None);
    }
    let backup = if found > 0 {
        Some(back_up(connection, backups, found)?)
    } else {
        None
    };
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite("starting the migration"))?;
    for migration in migrations
        .iter()
        .filter(|migration| migration.version > found)
    {
        transaction
            .execute_batch(migration.sql)
            .map_err(|source| StoreError::Migration {
                version: migration.version,
                source,
            })?;
    }
    transaction
        .execute(
            schema::META_SET,
            (MetaKey::SchemaVersion.name(), latest.to_string()),
        )
        .map_err(sqlite("recording the schema version"))?;
    transaction
        .commit()
        .map_err(sqlite("committing the migration"))?;
    Ok(backup)
}

/// The number of the last of `migrations`, 0 when there are none.
fn latest_version(migrations: &[Migration]) -> u32 {
    migrations.last().map_or(0, |migration| migration.version)
}

/// The database's schema version: 0 when it has no tables.
///
/// # Errors
///
/// [`StoreError::UnknownSchema`] when it has tables but no version Cortex wrote, and SQLite errors.
pub(crate) fn schema_version(connection: &Connection) -> Result<u32, StoreError> {
    let objects: i64 = connection
        .query_row(schema::COUNT_OBJECTS, [], |row| row.get(0))
        .map_err(sqlite("reading the schema"))?;
    if objects == 0 {
        return Ok(0);
    }
    let meta_tables: i64 = connection
        .query_row(schema::COUNT_META_TABLES, [], |row| row.get(0))
        .map_err(sqlite("reading the schema"))?;
    if meta_tables == 0 {
        return Err(StoreError::UnknownSchema);
    }
    let version: Option<String> = connection
        .query_row(schema::META_GET, [MetaKey::SchemaVersion.name()], |row| {
            row.get(0)
        })
        .optional()
        .map_err(sqlite("reading the schema version"))?;
    version
        .and_then(|text| text.parse().ok())
        .ok_or(StoreError::UnknownSchema)
}

/// Copies the database at schema `version` to `cortex-v<version>.db` in `backups`, durably, and
/// returns the copy's path. The copy is written beside its final name first and renamed into place
/// only when complete, so an interrupted copy never replaces a good one.
fn back_up(connection: &Connection, backups: &Path, version: u32) -> Result<PathBuf, StoreError> {
    private::create_folder(backups).map_err(io("creating the backups folder", backups))?;
    let target = backups.join(format!("cortex-v{version}.db"));
    let partial = backups.join(format!("cortex-v{version}.db.partial"));
    // An earlier attempt may have stopped part of the way; its copy is incomplete.
    match fs::remove_file(&partial) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(io("removing an unfinished backup", &partial)(error));
        }
        _ => {}
    }
    // Created empty and private first: SQLite copies into an empty file and keeps its mode.
    drop(private::open_file(&partial).map_err(io("creating a backup", &partial))?);
    connection
        .backup(MAIN_DB, &partial, None::<fn(rusqlite::backup::Progress)>)
        .map_err(sqlite("backing up the database"))?;
    // Opened for writing: Windows refuses to flush a file opened only for reading.
    fs::OpenOptions::new()
        .write(true)
        .open(&partial)
        .and_then(|copy| copy.sync_all())
        .map_err(io("flushing a backup", &partial))?;
    fs::rename(&partial, &target).map_err(io("placing a backup", &target))?;
    private::sync_folder(backups).map_err(io("flushing the backups folder", backups))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::{self, Access};
    use crate::schema::MIGRATIONS;

    /// The shipped first migration.
    const FIRST: Migration = schema::MIGRATIONS[0];
    /// A second migration that works.
    const SECOND: Migration = Migration {
        version: 2,
        sql: "CREATE TABLE extra (x INTEGER) STRICT;",
    };
    /// A second migration whose first statement works and whose second does not.
    const BROKEN: Migration = Migration {
        version: 2,
        sql: "CREATE TABLE extra (x INTEGER) STRICT; CREATE TABLE files (y INTEGER);",
    };

    /// A temporary folder with an empty database in it and an open connection.
    fn database() -> Result<(tempfile::TempDir, Connection), Box<dyn std::error::Error>> {
        let folder = tempfile::tempdir()?;
        let connection = connection::connect(&folder.path().join("cortex.db"), Access::ReadWrite)?;
        connection::configure(&connection, Access::ReadWrite, 256 * 1024 * 1024)?;
        Ok((folder, connection))
    }

    /// The names of the tables and indexes the database defines, sorted.
    fn objects(connection: &Connection) -> rusqlite::Result<Vec<String>> {
        let mut statement = connection.prepare("SELECT name FROM sqlite_schema ORDER BY name")?;
        statement.query_map([], |row| row.get(0))?.collect()
    }

    /// The files in `folder`, sorted; none when it does not exist.
    fn files_in(folder: &Path) -> std::io::Result<Vec<String>> {
        if !folder.try_exists()? {
            return Ok(Vec::new());
        }
        let mut names = std::fs::read_dir(folder)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }

    /// An empty database becomes the latest version, with no backup: there is nothing to keep.
    /// Running again changes nothing.
    #[test]
    fn empty_database_migrates_once() -> Result<(), Box<dyn std::error::Error>> {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        assert_eq!(schema_version(&connection)?, 0);
        assert_eq!(migrate(&mut connection, &[FIRST], &backups)?, None);
        assert_eq!(schema_version(&connection)?, 1);
        let before = objects(&connection)?;
        assert_eq!(migrate(&mut connection, &[FIRST], &backups)?, None);
        assert_eq!(objects(&connection)?, before);
        assert_eq!(files_in(&backups)?, Vec::<String>::new());
        Ok(())
    }

    /// A database of the first schema, with an index completed, gains the table of what files
    /// pass on and loses the record of the completed index, so that the next index reads every
    /// file and fills the table.
    #[test]
    fn the_second_schema_has_every_file_read_again() -> Result<(), Box<dyn std::error::Error>> {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        let first = MIGRATIONS.first().copied().ok_or("no migration")?;
        migrate(&mut connection, &[first], &backups)?;
        connection.execute(
            "INSERT INTO meta (key, value) VALUES ('cortex_version', '2.0.0')",
            [],
        )?;
        let second = MIGRATIONS.get(..2).ok_or("fewer than two migrations")?;
        migrate(&mut connection, second, &backups)?;
        assert_eq!(schema_version(&connection)?, 2);
        assert!(objects(&connection)?.contains(&"passes_on".to_owned()));
        let completed: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'cortex_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        assert_eq!(completed, None);
        Ok(())
    }

    /// A database of the second schema, with an index completed, gains the index of references
    /// through imports and keeps the record of the completed index: the new index is built from
    /// rows already there, so no file needs reading again.
    #[test]
    fn the_third_schema_keeps_the_index_complete() -> Result<(), Box<dyn std::error::Error>> {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        let second = MIGRATIONS.get(..2).ok_or("fewer than two migrations")?;
        migrate(&mut connection, second, &backups)?;
        connection.execute(
            "INSERT INTO meta (key, value) VALUES ('cortex_version', '2.0.0')",
            [],
        )?;
        let third = MIGRATIONS.get(..3).ok_or("fewer than three migrations")?;
        migrate(&mut connection, third, &backups)?;
        assert_eq!(schema_version(&connection)?, 3);
        assert!(objects(&connection)?.contains(&"refs_through_imports".to_owned()));
        let completed: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'cortex_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        assert_eq!(completed.as_deref(), Some("2.0.0"));
        Ok(())
    }

    /// A database of the third schema, with an index completed, gains the index of names passed on
    /// as others and keeps the record of the completed index: the new index is built from rows
    /// already there.
    #[test]
    fn the_fourth_schema_keeps_the_index_complete() -> Result<(), Box<dyn std::error::Error>> {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        let third = MIGRATIONS.get(..3).ok_or("fewer than three migrations")?;
        migrate(&mut connection, third, &backups)?;
        connection.execute(
            "INSERT INTO meta (key, value) VALUES ('cortex_version', '2.0.0')",
            [],
        )?;
        let fourth = MIGRATIONS.get(..4).ok_or("fewer than four migrations")?;
        migrate(&mut connection, fourth, &backups)?;
        assert_eq!(schema_version(&connection)?, 4);
        assert!(objects(&connection)?.contains(&"passes_on_aliases".to_owned()));
        let completed: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'cortex_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        assert_eq!(completed.as_deref(), Some("2.0.0"));
        Ok(())
    }

    /// A database of the fourth schema, with an index completed and a file stored, gains the
    /// tables of sessions, tasks and events, empty, and keeps the file and the record of the
    /// completed index: nothing the index wrote changes.
    #[test]
    fn the_fifth_schema_adds_activity_and_keeps_the_index() -> Result<(), Box<dyn std::error::Error>>
    {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        let fourth = MIGRATIONS.get(..4).ok_or("fewer than four migrations")?;
        migrate(&mut connection, fourth, &backups)?;
        connection.execute(
            "INSERT INTO meta (key, value) VALUES ('cortex_version', '2.0.0')",
            [],
        )?;
        connection.execute(
            "INSERT INTO files (path, bytes, mtime_ns, flags, indexed_at) \
             VALUES ('src/a.py', 10, 1, 0, 2)",
            [],
        )?;
        migrate(&mut connection, MIGRATIONS, &backups)?;
        assert_eq!(schema_version(&connection)?, 5);
        let found = objects(&connection)?;
        for table in [
            "sessions",
            "tasks",
            "events",
            "events_task",
            "events_session_ts",
        ] {
            assert!(found.contains(&table.to_owned()), "{table}");
        }
        let counts: (i64, i64, i64, i64) = connection.query_row(
            "SELECT (SELECT count(*) FROM sessions), (SELECT count(*) FROM tasks), \
             (SELECT count(*) FROM events), (SELECT count(*) FROM files)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(counts, (0, 0, 0, 1));
        let completed: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'cortex_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        assert_eq!(completed.as_deref(), Some("2.0.0"));
        Ok(())
    }

    /// A database with a schema is copied, as it was, before the next migration runs.
    #[test]
    fn existing_schema_is_backed_up_first() -> Result<(), Box<dyn std::error::Error>> {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        migrate(&mut connection, &[FIRST], &backups)?;
        connection.execute(
            "INSERT INTO meta (key, value) VALUES ('marker', 'kept')",
            [],
        )?;
        let backup = migrate(&mut connection, &[FIRST, SECOND], &backups)?;
        assert_eq!(backup, Some(backups.join("cortex-v1.db")));
        assert_eq!(files_in(&backups)?, vec!["cortex-v1.db".to_owned()]);
        assert_eq!(schema_version(&connection)?, 2);
        assert!(objects(&connection)?.contains(&"extra".to_owned()));
        let copy = Connection::open(backups.join("cortex-v1.db"))?;
        assert_eq!(schema_version(&copy)?, 1);
        assert!(!objects(&copy)?.contains(&"extra".to_owned()));
        let marker: String =
            copy.query_row("SELECT value FROM meta WHERE key = 'marker'", [], |row| {
                row.get(0)
            })?;
        assert_eq!(marker, "kept");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(backups.join("cortex-v1.db"))?
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        Ok(())
    }

    /// A migration that fails partway leaves the database exactly as it was, from an empty
    /// database and from an existing schema.
    #[test]
    fn a_failed_migration_changes_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        let error = migrate(&mut connection, &[FIRST, BROKEN], &backups).err();
        assert!(
            matches!(error, Some(StoreError::Migration { version: 2, .. })),
            "{error:?}"
        );
        assert_eq!(objects(&connection)?, Vec::<String>::new());
        assert_eq!(schema_version(&connection)?, 0);

        migrate(&mut connection, &[FIRST], &backups)?;
        let before = objects(&connection)?;
        let error = migrate(&mut connection, &[FIRST, BROKEN], &backups).err();
        assert!(
            matches!(error, Some(StoreError::Migration { version: 2, .. })),
            "{error:?}"
        );
        assert_eq!(objects(&connection)?, before);
        assert_eq!(schema_version(&connection)?, 1);
        Ok(())
    }

    /// Failed attempts leave one copy per schema version, not one per attempt, and a copy an
    /// earlier attempt left unfinished is replaced.
    #[test]
    fn repeated_attempts_keep_one_backup() -> Result<(), Box<dyn std::error::Error>> {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        migrate(&mut connection, &[FIRST], &backups)?;
        std::fs::create_dir_all(&backups)?;
        std::fs::write(backups.join("cortex-v1.db.partial"), b"cut short")?;
        for _ in 0..3 {
            let error = migrate(&mut connection, &[FIRST, BROKEN], &backups).err();
            assert!(
                matches!(error, Some(StoreError::Migration { version: 2, .. })),
                "{error:?}"
            );
        }
        assert_eq!(files_in(&backups)?, vec!["cortex-v1.db".to_owned()]);
        let copy = Connection::open(backups.join("cortex-v1.db"))?;
        assert_eq!(schema_version(&copy)?, 1);
        Ok(())
    }

    /// A database from a newer Cortex, or one Cortex did not write, is refused untouched.
    #[test]
    fn unknown_schemas_are_refused() -> Result<(), Box<dyn std::error::Error>> {
        let (folder, mut connection) = database()?;
        let backups = folder.path().join("backups");
        migrate(&mut connection, &[FIRST, SECOND], &backups)?;
        let error = migrate(&mut connection, &[FIRST], &backups).err();
        assert!(
            matches!(
                error,
                Some(StoreError::NewerSchema {
                    found: 2,
                    supported: 1
                })
            ),
            "{error:?}"
        );

        let (_other_folder, mut other) = database()?;
        other.execute("CREATE TABLE notes (text TEXT)", [])?;
        let error = migrate(&mut other, &[FIRST], &backups).err();
        assert!(
            matches!(error, Some(StoreError::UnknownSchema)),
            "{error:?}"
        );
        assert_eq!(objects(&other)?, vec!["notes".to_owned()]);
        Ok(())
    }
}
