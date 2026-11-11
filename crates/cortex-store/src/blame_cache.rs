//! The blame of a line range of a file, kept so that the same span is not blamed again: which
//! commit last changed each run of lines, for the file at a path as it stood when its content
//! hashed to a given value. A change to the file changes its hash, so a stored blame can only be
//! read back for the content it was made from; two files of the same content are blamed apart,
//! since different commits can have written them.
//!
//! One row holds a whole blame, its runs as JSON, because a blame is read and replaced as one: a
//! lookup is one read of the rows whose range overlaps the one asked for, and the caller combines
//! them. Only line numbers and commit ids are kept, never a line's text or a person's name.

use cortex_core::{ContentHash, RepoPath, is_object_id};
use rusqlite::Connection;

use crate::error::{StoreError, sqlite};
use crate::queries::Queries;
use crate::schema;
use crate::values;

/// A run of lines in a blame: lines that one commit last changed and that sit together in the
/// file as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameRunRecord {
    /// The first line of the run, counted from 1, in the file as it stands.
    pub start: u32,
    /// The last line of the run, inclusive, in the same file.
    pub end: u32,
    /// The first line's number in the commit's own file, counted from 1.
    pub source_start: u32,
    /// The commit that last changed these lines, as a hexadecimal object id.
    pub oid: String,
}

/// The blame of lines `first_line` to `last_line` of the file at `path` whose content hashed to
/// `file_hash`, and when it was computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameRecord {
    /// The file's path.
    pub path: RepoPath,
    /// The hash of the file's content the blame was made for.
    pub file_hash: ContentHash,
    /// The first line blamed, counted from 1.
    pub first_line: u32,
    /// The last line blamed, inclusive.
    pub last_line: u32,
    /// The runs, in file order.
    pub runs: Vec<BlameRunRecord>,
    /// When the blame was computed, in UTC milliseconds.
    pub computed_at_ms: i64,
}

impl Queries<'_> {
    /// The stored blames of the file at `path` whose content hashed to `file_hash` whose range of
    /// lines overlaps `first_line` to `last_line` (the edges count), by first line and then last.
    /// A range may be covered by several blames, or only in part; the caller combines them.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored runs that cannot be read back.
    pub fn cached_blames(
        self,
        (path, file_hash): (&RepoPath, &ContentHash),
        first_line: u32,
        last_line: u32,
    ) -> Result<Vec<BlameRecord>, StoreError> {
        let rows: Vec<(i64, i64, String, i64)> = self
            .connection()
            .prepare_cached(schema::BLAME_OVERLAPPING)
            .and_then(|mut statement| {
                statement
                    .query_map(
                        (
                            path.as_str(),
                            file_hash.as_bytes().as_slice(),
                            first_line,
                            last_line,
                        ),
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )?
                    .collect()
            })
            .map_err(sqlite("reading cached blames"))?;
        rows.into_iter()
            .map(|(first, last, runs, computed_at_ms)| {
                Ok(BlameRecord {
                    path: path.clone(),
                    file_hash: *file_hash,
                    first_line: values::number(first, "blame_cache.first_line", first)?,
                    last_line: values::number(last, "blame_cache.last_line", first)?,
                    runs: read_runs(&runs, first)?,
                    computed_at_ms,
                })
            })
            .collect()
    }

    /// How many blames are kept.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn cached_blame_count(self) -> Result<u64, StoreError> {
        let count: i64 = self
            .connection()
            .query_row(schema::BLAME_ROWS, [], |row| row.get(0))
            .map_err(sqlite("counting cached blames"))?;
        values::number(count, "blame_cache.count", count)
    }
}

/// Stores `record`, replacing the blame of the same file content and range.
pub(crate) fn put(connection: &Connection, record: &BlameRecord) -> Result<(), StoreError> {
    let invalid = |column| StoreError::Invalid { column };
    if record.first_line == 0 || record.first_line > record.last_line {
        return Err(invalid("blame_cache.first_line"));
    }
    if record.runs.iter().any(|run| !is_object_id(&run.oid)) {
        return Err(invalid("blame_cache.runs"));
    }
    let runs: Vec<(u32, u32, u32, &str)> = record
        .runs
        .iter()
        .map(|run| (run.start, run.end, run.source_start, run.oid.as_str()))
        .collect();
    let runs = serde_json::to_string(&runs).map_err(|_| invalid("blame_cache.runs"))?;
    connection
        .prepare_cached(schema::BLAME_PUT)
        .and_then(|mut statement| {
            statement.execute((
                record.path.as_str(),
                record.file_hash.as_bytes().as_slice(),
                record.first_line,
                record.last_line,
                runs,
                record.computed_at_ms,
            ))
        })
        .map(drop)
        .map_err(sqlite("writing a cached blame"))
}

/// The runs `text` holds, for the blame whose first line is `row`.
fn read_runs(text: &str, row: i64) -> Result<Vec<BlameRunRecord>, StoreError> {
    let corrupt = || StoreError::Corrupt {
        column: "blame_cache.runs",
        row,
    };
    let runs: Vec<(u32, u32, u32, String)> = serde_json::from_str(text).map_err(|_| corrupt())?;
    runs.into_iter()
        .map(|(start, end, source_start, oid)| {
            if !is_object_id(&oid) {
                return Err(corrupt());
            }
            Ok(BlameRunRecord {
                start,
                end,
                source_start,
                oid,
            })
        })
        .collect()
}
