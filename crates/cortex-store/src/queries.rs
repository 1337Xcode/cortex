//! Read queries, shared by readers and by the writer's transactions.

use cortex_core::{Identifier, NodeKind, RepoPath};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::{FileId, NodeId, SymbolId};
use crate::meta::MetaKey;
use crate::raw::{RawEdge, RawFile, RawRef, RawSymbol};
use crate::rows::{EdgeRow, FileRow, IndexCounts, RefRow, SymbolRow};
use crate::schema;
use crate::values;

/// Read queries over one connection. A [`crate::WriteTx`]'s queries also see its own changes.
#[derive(Debug, Clone, Copy)]
pub struct Queries<'c> {
    /// The connection the queries run on.
    connection: &'c Connection,
}

impl<'c> Queries<'c> {
    /// Queries over `connection`.
    pub(crate) fn new(connection: &'c Connection) -> Self {
        Self { connection }
    }

    /// The value stored under `key`.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn meta(self, key: MetaKey) -> Result<Option<String>, StoreError> {
        self.connection
            .query_row(schema::META_GET, [key.name()], |row| row.get(0))
            .optional()
            .map_err(sqlite("reading meta"))
    }

    /// The file at `path`.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn file(self, path: &RepoPath) -> Result<Option<FileRow>, StoreError> {
        let raw = self
            .connection
            .prepare_cached(schema::FILE_BY_PATH)
            .and_then(|mut statement| {
                statement
                    .query_row([path.as_str()], RawFile::read)
                    .optional()
            })
            .map_err(sqlite("reading a file"))?;
        raw.map(RawFile::check).transpose()
    }

    /// The path of every stored file, in the order [`RepoPath`] sorts them: by the bytes of the
    /// path.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored paths that cannot be read back.
    pub fn file_paths(self) -> Result<Vec<RepoPath>, StoreError> {
        let raw: Vec<(i64, String)> = self
            .connection
            .prepare_cached(schema::FILE_PATHS)
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .map_err(sqlite("reading file paths"))?;
        raw.iter()
            .map(|(id, path)| values::path(path, "files.path", *id))
            .collect()
    }

    /// The symbols defined in `file`, in the order they appear.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn symbols_in_file(self, file: FileId) -> Result<Vec<SymbolRow>, StoreError> {
        self.symbols(schema::SYMBOLS_IN_FILE, file.get())
    }

    /// The symbols named `name`, oldest first.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn symbols_named(self, name: &Identifier) -> Result<Vec<SymbolRow>, StoreError> {
        self.symbols(schema::SYMBOLS_NAMED, name.as_str())
    }

    /// The references in `file`, in the order they were written.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn refs_in_file(self, file: FileId) -> Result<Vec<RefRow>, StoreError> {
        let raw = self
            .collect(schema::REFS_IN_FILE, file.get(), RawRef::read)
            .map_err(sqlite("reading references"))?;
        raw.into_iter().map(RawRef::check).collect()
    }

    /// The node of `file`.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn file_node(self, file: FileId) -> Result<Option<NodeId>, StoreError> {
        self.node_of(NodeKind::File, file.get())
    }

    /// The node of `symbol`.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn symbol_node(self, symbol: SymbolId) -> Result<Option<NodeId>, StoreError> {
        self.node_of(NodeKind::Symbol, symbol.get())
    }

    /// The edges that start at `node`, by type, then target.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn edges_from(self, node: NodeId) -> Result<Vec<EdgeRow>, StoreError> {
        let raw = self
            .collect(schema::EDGES_FROM, node.get(), RawEdge::read)
            .map_err(sqlite("reading edges"))?;
        raw.into_iter().map(RawEdge::check).collect()
    }

    /// How many rows the index holds.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn counts(self) -> Result<IndexCounts, StoreError> {
        let raw: [i64; 6] = self
            .connection
            .query_row(schema::COUNTS, [], |row| {
                Ok([
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ])
            })
            .map_err(sqlite("counting rows"))?;
        let [files, symbols, refs, resolved_refs, nodes, edges] =
            raw.map(|count| values::number::<u64>(count, "count", 0));
        Ok(IndexCounts {
            files: files?,
            symbols: symbols?,
            refs: refs?,
            resolved_refs: resolved_refs?,
            nodes: nodes?,
            edges: edges?,
        })
    }

    /// The node of the row `id` of the table `kind` names.
    fn node_of(self, kind: NodeKind, id: i64) -> Result<Option<NodeId>, StoreError> {
        self.connection
            .prepare_cached(schema::NODE_OF)
            .and_then(|mut statement| {
                statement
                    .query_row((kind.name(), id), |row| row.get(0))
                    .optional()
            })
            .map(|node| node.map(NodeId))
            .map_err(sqlite("reading a node"))
    }

    /// The symbols `sql` selects with `key`.
    fn symbols(
        self,
        sql: &'static str,
        key: impl rusqlite::ToSql,
    ) -> Result<Vec<SymbolRow>, StoreError> {
        let raw = self
            .collect(sql, key, RawSymbol::read)
            .map_err(sqlite("reading symbols"))?;
        raw.into_iter().map(RawSymbol::check).collect()
    }

    /// Every row `sql` selects with `key`, read by `read`.
    fn collect<T>(
        self,
        sql: &'static str,
        key: impl rusqlite::ToSql,
        read: fn(&Row<'_>) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<Vec<T>> {
        let mut statement = self.connection.prepare_cached(sql)?;
        statement.query_map([key], read)?.collect()
    }
}
