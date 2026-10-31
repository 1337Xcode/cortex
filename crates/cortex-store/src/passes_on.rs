//! The `passes_on` rows: what each file passes on of what its top-level imports bring in, written
//! with the file and read back for the files a name is followed through.
//!
//! A [`PassedOn::Name`] is one row per file its import names, or a single row without a path when
//! the import names something outside the repository; [`PassedOn::Every`] is a row without a name;
//! [`PassedOn::Outside`] is a row without a name or a path.

use std::collections::BTreeMap;

use cortex_core::{Identifier, RepoPath};
use rusqlite::{Connection, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::FileId;
use crate::records::PassedOn;
use crate::schema;
use crate::values;

/// A `passes_on` row as SQLite returns it.
struct RawPassedOn {
    /// `file_id`.
    file: i64,
    /// `position`.
    position: Option<i64>,
    /// `name`.
    name: Option<String>,
    /// `taken`.
    taken: Option<String>,
    /// `path`.
    path: Option<String>,
}

impl RawPassedOn {
    /// Reads the columns of [`schema::PASSES_ON_OF_FILE`] and [`schema::PASSES_ON_EVERY`].
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            file: row.get(0)?,
            position: row.get(1)?,
            name: row.get(2)?,
            taken: row.get(3)?,
            path: row.get(4)?,
        })
    }
}

/// Replaces what the file `file` passes on with `passed`.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn write(
    connection: &Connection,
    file: FileId,
    passed: &[PassedOn],
) -> Result<(), StoreError> {
    delete(connection, file)?;
    let mut statement = connection
        .prepare_cached(schema::PASSES_ON_INSERT)
        .map_err(sqlite("recording what a file passes on"))?;
    let mut insert = |position: Option<u32>, name: Option<&str>, taken: Option<&str>, path| {
        statement
            .execute((file.get(), position, name, taken, path))
            .map(drop)
            .map_err(sqlite("recording what a file passes on"))
    };
    for passed in passed {
        match passed {
            PassedOn::Name {
                position,
                name,
                taken,
                files,
            } => {
                let (name, taken) = (Some(name.as_str()), Some(taken.as_str()));
                if files.is_empty() {
                    insert(Some(*position), name, taken, None)?;
                }
                for path in files {
                    insert(Some(*position), name, taken, Some(path.as_str()))?;
                }
            }
            PassedOn::Every(path) => insert(None, None, None, Some(path.as_str()))?,
            PassedOn::Outside => insert(None, None, None, None)?,
        }
    }
    Ok(())
}

/// The names files pass `taken` on as, other than `taken` itself, in name order.
///
/// # Errors
///
/// SQLite errors and stored names that cannot be read back.
pub(crate) fn aliases(
    connection: &Connection,
    taken: &Identifier,
) -> Result<Vec<Identifier>, StoreError> {
    let raw: Vec<(String, i64)> = connection
        .prepare_cached(schema::PASSES_ON_ALIASES)
        .and_then(|mut statement| {
            statement
                .query_map([taken.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(sqlite("reading the names a name is passed on as"))?;
    raw.iter()
        .map(|(name, row)| values::identifier(name, "passes_on.name", *row))
        .collect()
}

/// Deletes what the file `file` passes on.
///
/// # Errors
///
/// SQLite errors.
pub(crate) fn delete(connection: &Connection, file: FileId) -> Result<(), StoreError> {
    connection
        .prepare_cached(schema::PASSES_ON_DELETE)
        .and_then(|mut statement| statement.execute([file.get()]))
        .map(drop)
        .map_err(sqlite("deleting what a file passes on"))
}

/// What the file `file` passes on, in the order it was written.
///
/// # Errors
///
/// SQLite errors and stored values that cannot be read back.
pub(crate) fn of_file(connection: &Connection, file: FileId) -> Result<Vec<PassedOn>, StoreError> {
    let raw: Vec<RawPassedOn> = connection
        .prepare_cached(schema::PASSES_ON_OF_FILE)
        .and_then(|mut statement| {
            statement
                .query_map([file.get()], RawPassedOn::read)?
                .collect()
        })
        .map_err(sqlite("reading what a file passes on"))?;
    Ok(group(raw)?.remove(&file).unwrap_or_default())
}

/// What every file that passes anything on passes on, by file.
///
/// # Errors
///
/// SQLite errors and stored values that cannot be read back.
pub(crate) fn of_every_file(
    connection: &Connection,
) -> Result<BTreeMap<FileId, Vec<PassedOn>>, StoreError> {
    let raw: Vec<RawPassedOn> = connection
        .prepare_cached(schema::PASSES_ON_EVERY)
        .and_then(|mut statement| statement.query_map([], RawPassedOn::read)?.collect())
        .map_err(sqlite("reading what files pass on"))?;
    group(raw)
}

/// The rows `raw`, in the order they were written, as what each file passes on: the rows of one
/// import's name joined into one [`PassedOn::Name`].
fn group(raw: Vec<RawPassedOn>) -> Result<BTreeMap<FileId, Vec<PassedOn>>, StoreError> {
    let mut grouped: BTreeMap<FileId, Vec<PassedOn>> = BTreeMap::new();
    for row in raw {
        let file = FileId(row.file);
        let passed = grouped.entry(file).or_default();
        let path = row
            .path
            .map(|path| values::path(&path, "passes_on.path", row.file))
            .transpose()?;
        let (Some(position), Some(name), Some(taken)) = (row.position, row.name, row.taken) else {
            passed.push(path.map_or(PassedOn::Outside, PassedOn::Every));
            continue;
        };
        let position: u32 = values::number(position, "passes_on.position", row.file)?;
        let name = values::identifier(&name, "passes_on.name", row.file)?;
        let taken = values::identifier(&taken, "passes_on.taken", row.file)?;
        if let Some(PassedOn::Name {
            position: last,
            name: last_name,
            taken: last_taken,
            files,
        }) = passed.last_mut()
            && *last == position
            && *last_name == name
            && *last_taken == taken
        {
            files.extend(path);
            continue;
        }
        passed.push(PassedOn::Name {
            position,
            name,
            taken,
            files: path.into_iter().collect::<Vec<RepoPath>>(),
        });
    }
    Ok(grouped)
}
