//! The one writer of a repository's database, and its transactions.

use std::path::Path;

use cortex_core::{EdgeType, RepoId, RepoPath};
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::connection::{self, Access};
use crate::error::{StoreError, io, sqlite};
use crate::folders;
use crate::ids::NodeId;
use crate::layout;
use crate::links;
use crate::lock::WriterLock;
use crate::meta::MetaKey;
use crate::migrate;
use crate::private;
use crate::queries::Queries;
use crate::records::{EdgeRecord, FileChange, FileIndex, FileRemoval, Resolved};
use crate::replace;
use crate::schema;

/// What [`Writer::open`] needs.
#[derive(Debug, Clone, Copy)]
pub struct WriterOptions<'a> {
    /// The repository's `.cortex` folder, created when missing.
    pub folder: &'a Path,
    /// The repository's id, recorded in `meta`.
    pub repo_id: &'a RepoId,
    /// How much of the database SQLite may map into memory (`store.mmap_bytes`).
    pub mmap_bytes: u64,
    /// The time in UTC milliseconds, recorded as a new database's creation time.
    pub now_ms: i64,
}

/// The one writer of a repository's database. While it exists, no other writer can open the
/// same database, in this process or any other.
#[derive(Debug)]
pub struct Writer {
    // Fields drop in order: the connection closes before the lock is released, so a new writer
    // never opens the database while this one is still closing it.
    /// The read-write connection.
    connection: Connection,
    /// The lock that makes this the only writer.
    _lock: WriterLock,
}

impl Writer {
    /// Takes the writer lock, opens the database and brings its schema up to date, backing it up
    /// first when it already has one.
    ///
    /// # Errors
    ///
    /// [`StoreError::WriterBusy`] when another writer has the database, the schema errors of a
    /// migration, and I/O and SQLite errors.
    pub fn open(options: &WriterOptions<'_>) -> Result<Self, StoreError> {
        private::create_folder(options.folder)
            .map_err(io("creating the store folder", options.folder))?;
        let lock = WriterLock::acquire(&layout::writer_lock(options.folder))?;
        let database = layout::database(options.folder);
        // Created here, private, before SQLite creates it with the default permissions.
        drop(private::open_file(&database).map_err(io("creating the database", &database))?);
        let mut connection = connection::connect(&database, Access::ReadWrite)?;
        // Checked before any setting is applied: a database Cortex did not write, or one a newer
        // Cortex wrote, is left exactly as it was.
        migrate::check(&connection, schema::MIGRATIONS)?;
        connection::configure(&connection, Access::ReadWrite, options.mmap_bytes)?;
        migrate::migrate(
            &mut connection,
            schema::MIGRATIONS,
            &layout::backups(options.folder),
        )?;
        record_identity(&mut connection, options)?;
        Ok(Self {
            connection,
            _lock: lock,
        })
    }

    /// Starts a transaction. Only one can be open at a time.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn transaction(&mut self) -> Result<WriteTx<'_>, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite("starting a transaction"))?;
        Ok(WriteTx { transaction })
    }

    /// Read queries over the committed database.
    #[must_use]
    pub fn queries(&self) -> Queries<'_> {
        Queries::new(&self.connection)
    }
}

/// Records which repository and which Cortex the database belongs to, and when it was created.
fn record_identity(
    connection: &mut Connection,
    options: &WriterOptions<'_>,
) -> Result<(), StoreError> {
    let transaction = connection
        .transaction()
        .map_err(sqlite("starting a transaction"))?;
    let entries = [
        (
            schema::META_SET,
            MetaKey::RepoId,
            options.repo_id.as_str().to_owned(),
        ),
        (
            schema::META_SET,
            MetaKey::CortexVersion,
            env!("CARGO_PKG_VERSION").to_owned(),
        ),
        (
            schema::META_SET_ONCE,
            MetaKey::CreatedAt,
            options.now_ms.to_string(),
        ),
    ];
    for (sql, key, value) in entries {
        transaction
            .execute(sql, (key.name(), value))
            .map_err(sqlite("writing meta"))?;
    }
    transaction.commit().map_err(sqlite("writing meta"))
}

/// One transaction of changes. Readers see none of them until [`WriteTx::commit`]; dropping the
/// transaction without committing undoes them all. Each write is all or nothing on its own: when
/// one fails, its partial changes are undone and the transaction keeps the earlier writes.
#[derive(Debug)]
pub struct WriteTx<'w> {
    /// The open transaction.
    transaction: Transaction<'w>,
}

impl WriteTx<'_> {
    /// Writes everything the index records about one file over what was stored for it, keeping
    /// the ids of symbols that match one already stored by qualified name and kind. The file's
    /// references are written unresolved; the file's and each symbol's node get their search
    /// rows, and the nodes of symbols no longer defined lose theirs.
    ///
    /// # Errors
    ///
    /// [`StoreError::Position`] when a symbol's parent does not come before it or a reference
    /// names a symbol the file does not have, [`StoreError::TransactionLost`], and SQLite errors.
    pub fn replace_file(&mut self, index: &FileIndex) -> Result<FileChange, StoreError> {
        self.atomically(|connection| replace::replace_file(connection, index))
    }

    /// Deletes everything stored for the file at `path`, its nodes' search rows included.
    /// References elsewhere to its symbols become unresolved. `None` when nothing was stored.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_file(&mut self, path: &RepoPath) -> Result<Option<FileRemoval>, StoreError> {
        self.atomically(|connection| replace::remove_file(connection, path))
    }

    /// Ties each reference to its target, or marks it unresolved; the confidence follows from
    /// the resolution.
    ///
    /// # Errors
    ///
    /// [`StoreError::UnknownRow`] for a reference that does not exist,
    /// [`StoreError::TransactionLost`], and SQLite errors, including a target symbol that does not
    /// exist. Either all resolutions are written or none.
    pub fn set_resolutions(&mut self, resolutions: &[Resolved]) -> Result<(), StoreError> {
        self.atomically(|connection| links::set_resolutions(connection, resolutions))
    }

    /// Replaces the edges of `edge_type` that start at `src` with `edges`.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors, including a node that does not exist and
    /// two edges to the same node. Either all edges are written or none.
    pub fn replace_edges(
        &mut self,
        src: NodeId,
        edge_type: EdgeType,
        edges: &[EdgeRecord],
    ) -> Result<(), StoreError> {
        self.atomically(|connection| links::replace_edges(connection, src, edge_type, edges))
    }

    /// The node of the folder at `path`, added when it has none, with its search row. A folder
    /// keeps its node, and everything attached to it, until the node is removed.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn folder_node(&mut self, path: &RepoPath) -> Result<NodeId, StoreError> {
        self.atomically(|connection| folders::folder_node(connection, path))
    }

    /// Deletes the node of the folder at `path`, its search row and every edge at it. `false` when
    /// the folder had no node.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] and SQLite errors.
    pub fn remove_folder_node(&mut self, path: &RepoPath) -> Result<bool, StoreError> {
        self.atomically(|connection| folders::remove_folder_node(connection, path))
    }

    /// Read queries that also see this transaction's changes.
    #[must_use]
    pub fn queries(&self) -> Queries<'_> {
        Queries::new(&self.transaction)
    }

    /// Runs `write` inside a savepoint, so that a write that fails part of the way through
    /// leaves the transaction as it was before it.
    fn atomically<T>(
        &mut self,
        write: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.refuse_if_lost()?;
        let savepoint = self
            .transaction
            .savepoint()
            .map_err(sqlite("starting a savepoint"))?;
        let value = write(&savepoint)?;
        savepoint
            .commit()
            .map_err(sqlite("releasing a savepoint"))?;
        Ok(value)
    }

    /// Refuses to go on when SQLite has rolled the transaction back on its own, as it may after a
    /// full disk, an I/O error or running out of memory. A savepoint begun then would start a new
    /// transaction of its own and commit each later write by itself.
    fn refuse_if_lost(&self) -> Result<(), StoreError> {
        if self.transaction.is_autocommit() {
            Err(StoreError::TransactionLost)
        } else {
            Ok(())
        }
    }

    /// Makes every change visible to readers.
    ///
    /// # Errors
    ///
    /// [`StoreError::TransactionLost`] when SQLite already rolled the transaction back, and SQLite
    /// errors; either way nothing was written.
    pub fn commit(self) -> Result<(), StoreError> {
        self.refuse_if_lost()?;
        self.transaction
            .commit()
            .map_err(sqlite("committing a transaction"))
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::{FileFlags, RepoId};

    use super::*;
    use crate::records::FileRecord;

    /// A file with no symbols or references at `path`.
    fn empty_file(path: &str) -> Result<FileIndex, Box<dyn std::error::Error>> {
        Ok(FileIndex {
            file: FileRecord {
                path: RepoPath::new(path)?,
                language: None,
                size_bytes: 1,
                line_count: None,
                content_hash: None,
                mtime_ns: 1,
                flags: FileFlags::empty(),
                indexed_at_ms: 1,
            },
            symbols: Vec::new(),
            refs: Vec::new(),
        })
    }

    /// Once SQLite has rolled a transaction back on its own, as it may after a full disk, later
    /// writes and the commit are refused, and nothing written after the rollback becomes visible.
    #[test]
    fn a_lost_transaction_refuses_more_writes() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let repo_id = RepoId::of(root.path());
        let mut writer = Writer::open(&WriterOptions {
            folder: &root.path().join(".cortex"),
            repo_id: &repo_id,
            mmap_bytes: 1 << 20,
            now_ms: 1,
        })?;
        let mut transaction = writer.transaction()?;
        transaction.replace_file(&empty_file("a.py")?)?;
        // What SQLite does to the transaction after such an error.
        transaction.transaction.execute_batch("ROLLBACK")?;
        let error = transaction.replace_file(&empty_file("b.py")?).err();
        assert!(
            matches!(error, Some(StoreError::TransactionLost)),
            "{error:?}"
        );
        let error = transaction.commit().err();
        assert!(
            matches!(error, Some(StoreError::TransactionLost)),
            "{error:?}"
        );
        assert_eq!(writer.queries().counts()?.files, 0);
        Ok(())
    }
}
