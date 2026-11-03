//! What the co-change miner keeps: each file's commit counts (`file_commits`), each pair's
//! co-change counts (`cochange_stats`) and the miner's own state (`git_state`). Its edges are the
//! graph's co-change edges, written as other edges are. Counts hold numbers and ids only.

use cortex_core::{FileFlags, RepoPath};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::{FileId, NodeId};
use crate::queries::{Queries, json_array};
use crate::schema;

/// A value of the co-change miner's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitStateKey {
    /// The newest commit mined, in hexadecimal.
    LastCommit,
    /// That commit's sequence number: the commits of a full mining are numbered from 1, the
    /// oldest, and the commits added later go on from the newest.
    LastSeq,
    /// When the last full mining was written, in UTC milliseconds.
    FullAt,
    /// The sequence number every commit is weighed from: the newest commit of the last full
    /// mining.
    AnchorSeq,
}

impl GitStateKey {
    /// The stored key.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::LastCommit => "last_commit",
            Self::LastSeq => "last_seq",
            Self::FullAt => "full_at",
            Self::AnchorSeq => "anchor_seq",
        }
    }
}

/// A file's commit counts.
#[derive(Debug, Clone, PartialEq)]
pub struct FileCommitsRow {
    /// The file.
    pub file: FileId,
    /// How many commits changed it.
    pub count: u32,
    /// Those commits' weights, summed.
    pub weighted: f64,
}

/// Two files' co-change counts, the file with the smaller id first.
#[derive(Debug, Clone, PartialEq)]
pub struct PairRow {
    /// The file with the smaller id.
    pub file_a: FileId,
    /// The file with the larger id.
    pub file_b: FileId,
    /// How many commits changed both.
    pub count: u32,
    /// Those commits' weights, summed.
    pub weighted: f64,
    /// The sequence number of the newest of those commits.
    pub last_commit_seq: u64,
}

/// A file that counts for co-change, neither generated nor vendored: its path, row and node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountedFile {
    /// The file's path.
    pub path: RepoPath,
    /// The file's row.
    pub file: FileId,
    /// The file's node.
    pub node: NodeId,
}

/// Deletes every file's and pair's counts.
pub(crate) fn clear(connection: &Connection) -> Result<(), StoreError> {
    for (sql, what) in [
        (schema::FILE_COMMITS_DELETE_ALL, "deleting commit counts"),
        (
            schema::COCHANGE_STATS_DELETE_ALL,
            "deleting co-change counts",
        ),
    ] {
        connection.execute(sql, []).map_err(sqlite(what))?;
    }
    Ok(())
}

/// Adds `files` and `pairs` to the stored counts, storing those not stored yet.
pub(crate) fn add(
    connection: &Connection,
    files: &[FileCommitsRow],
    pairs: &[PairRow],
) -> Result<(), StoreError> {
    let mut statement = connection
        .prepare_cached(schema::FILE_COMMITS_ADD)
        .map_err(sqlite("adding commit counts"))?;
    for row in files {
        statement
            .execute((row.file.get(), row.count, row.weighted))
            .map_err(sqlite("adding commit counts"))?;
    }
    let mut statement = connection
        .prepare_cached(schema::COCHANGE_STATS_ADD)
        .map_err(sqlite("adding co-change counts"))?;
    for row in pairs {
        let seq = i64::try_from(row.last_commit_seq).map_err(|_| StoreError::TooLarge {
            column: "cochange_stats.last_commit_seq",
            value: row.last_commit_seq,
        })?;
        statement
            .execute((
                row.file_a.get(),
                row.file_b.get(),
                row.count,
                row.weighted,
                seq,
            ))
            .map_err(sqlite("adding co-change counts"))?;
    }
    Ok(())
}

/// Sets the miner's state under `key` to `value`.
pub(crate) fn set_state(
    connection: &Connection,
    key: GitStateKey,
    value: &str,
) -> Result<(), StoreError> {
    connection
        .execute(schema::GIT_STATE_SET, (key.name(), value))
        .map(drop)
        .map_err(sqlite("writing the git state"))
}

/// Deletes the miner's state under `key`.
pub(crate) fn remove_state(connection: &Connection, key: GitStateKey) -> Result<(), StoreError> {
    connection
        .execute(schema::GIT_STATE_DELETE, [key.name()])
        .map(drop)
        .map_err(sqlite("deleting the git state"))
}

impl Queries<'_> {
    /// The co-change miner's state under `key`.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn git_state(self, key: GitStateKey) -> Result<Option<String>, StoreError> {
        self.connection()
            .query_row(schema::GIT_STATE_GET, [key.name()], |row| row.get(0))
            .optional()
            .map_err(sqlite("reading the git state"))
    }

    /// Every file that counts for co-change, neither generated nor vendored, by path.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn cochange_files(self) -> Result<Vec<CountedFile>, StoreError> {
        let skipped = FileFlags::GENERATED.with(FileFlags::VENDORED);
        let mut statement = self
            .connection()
            .prepare_cached(schema::COCHANGE_FILES)
            .map_err(sqlite("reading the files that count"))?;
        let rows = statement
            .query_map([skipped.bits()], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get(2)?))
            })
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading the files that count"))?;
        rows.into_iter()
            .map(|(file, path, node)| {
                let path = RepoPath::new(&path).map_err(|_| StoreError::Corrupt {
                    column: "files.path",
                    row: file,
                })?;
                Ok(CountedFile {
                    path,
                    file: FileId(file),
                    node: NodeId(node),
                })
            })
            .collect()
    }

    /// The commit counts of `files` that have some, by file.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn file_commits_of(self, files: &[FileId]) -> Result<Vec<FileCommitsRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::FILE_COMMITS_OF)
            .map_err(sqlite("reading commit counts"))?;
        statement
            .query_map([ids(files)], read_file_commits)
            .and_then(Iterator::collect)
            .map_err(sqlite("reading commit counts"))
    }

    /// The co-change counts of every pair that holds one of `files`, by pair.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn cochange_pairs_of(self, files: &[FileId]) -> Result<Vec<PairRow>, StoreError> {
        let mut statement = self
            .connection()
            .prepare_cached(schema::COCHANGE_STATS_OF)
            .map_err(sqlite("reading co-change counts"))?;
        let rows = statement
            .query_map([ids(files)], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(sqlite("reading co-change counts"))?;
        rows.into_iter()
            .map(|(file_a, file_b, count, weighted, seq)| {
                let last_commit_seq = u64::try_from(seq).map_err(|_| StoreError::Corrupt {
                    column: "cochange_stats.last_commit_seq",
                    row: file_a,
                })?;
                Ok(PairRow {
                    file_a: FileId(file_a),
                    file_b: FileId(file_b),
                    count,
                    weighted,
                    last_commit_seq,
                })
            })
            .collect()
    }
}

/// A row of `file_commits`.
fn read_file_commits(row: &Row<'_>) -> rusqlite::Result<FileCommitsRow> {
    Ok(FileCommitsRow {
        file: FileId(row.get(0)?),
        count: row.get(1)?,
        weighted: row.get(2)?,
    })
}

/// `files`' ids as a JSON array, the form `json_each` reads a list of bound values from.
fn ids(files: &[FileId]) -> String {
    json_array(files.iter().map(|file| file.get().to_string()))
}
