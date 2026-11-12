//! How much of `.cortex/` the history features' tables take: the bytes of the pages SQLite's
//! `dbstat` table gives each of `blame_cache`, `change_cards` and `commit_terms` (its full-text
//! index's own tables and every index of the three included), against the bytes of every file in
//! the folder. Read with the daemon stopped, so that its write-ahead log is folded into the
//! database and nothing writes while the pages are counted. The bundled SQLite is built with
//! `dbstat` (`libsqlite3-sys` passes `SQLITE_ENABLE_DBSTAT_VTAB`); a build without it fails the
//! query, and so the benchmark.

use std::fmt;
use std::path::Path;

use crate::common::TestResult;

/// The history features' tables.
const TABLES: [&str; 3] = ["blame_cache", "change_cards", "commit_terms"];

/// The bytes of the pages of a table: its own, those of its indexes, and those of the tables a
/// full-text table keeps its index in (`commit_terms_data` and the like). A table's indexes carry
/// its name as `tbl_name`; an FTS5 table's own tables are named after it with a `_` suffix.
const TABLE_BYTES: &str = "SELECT coalesce(sum(stat.pgsize), 0) FROM dbstat AS stat \
     JOIN sqlite_schema AS item ON stat.name = item.name \
     WHERE item.tbl_name = ?1 OR substr(item.tbl_name, 1, length(?1) + 1) = ?1 || '_'";

/// The history tables' bytes, each and in all, and the bytes of the folder they are in.
#[derive(Debug)]
pub(super) struct Shares {
    /// Each history table and its bytes.
    pub(super) tables: Vec<(&'static str, u64)>,
    /// The bytes of every file in `.cortex/`.
    pub(super) folder_bytes: u64,
}

impl Shares {
    /// The history tables' bytes together.
    pub(super) fn history_bytes(&self) -> u64 {
        self.tables.iter().map(|(_, bytes)| bytes).sum()
    }

    /// The share of the folder `bytes` make, from 0 to 1.
    #[allow(
        clippy::cast_precision_loss,
        reason = "byte counts of a benchmark's store are far below 2^52"
    )]
    pub(super) fn share_of(&self, bytes: u64) -> f64 {
        bytes as f64 / self.folder_bytes.max(1) as f64
    }
}

impl fmt::Display for Shares {
    /// Each table's bytes and share, then theirs together and the folder's bytes.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (table, bytes) in &self.tables {
            let share = self.share_of(*bytes) * 100.0;
            write!(formatter, "{table} {bytes} bytes ({share:.3}%), ")?;
        }
        let together = self.share_of(self.history_bytes()) * 100.0;
        write!(
            formatter,
            "together {together:.3}% of {} bytes",
            self.folder_bytes
        )
    }
}

/// The bytes of every file under `folder`, its subfolders included.
fn folder_bytes(folder: &Path) -> TestResult<u64> {
    let mut bytes = 0;
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        bytes += if kind.is_dir() {
            folder_bytes(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(bytes)
}

/// The shares of the history tables in the store at `folder`, a `.cortex` folder whose daemon is
/// stopped. The folder is measured before the database is opened, so that the files opening it
/// makes are not counted.
///
/// # Errors
///
/// I/O errors, or SQLite's: a build without `dbstat` among them.
pub(super) fn measure(folder: &Path) -> TestResult<Shares> {
    let folder_bytes = folder_bytes(folder)?;
    let store = rusqlite::Connection::open(folder.join("cortex.db"))?;
    let mut tables = Vec::new();
    for table in TABLES {
        let bytes: i64 = store.query_row(TABLE_BYTES, [table], |row| row.get(0))?;
        tables.push((table, u64::try_from(bytes)?));
    }
    Ok(Shares {
        tables,
        folder_bytes,
    })
}
