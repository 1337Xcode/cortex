//! Read queries, shared by readers and by the writer's transactions.

use cortex_core::{EdgeType, FileFlags, Identifier, NodeKind, RepoPath};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::{FileId, NodeId, SymbolId};
use crate::meta::MetaKey;
use crate::raw::{RawEdge, RawFile, RawRef, RawRefName, RawSymbol, RawSymbolName};
use crate::rows::{EdgeRow, FileRow, IndexCounts, RefName, RefRow, SymbolName, SymbolRow};
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

    /// Every stored file's row, by path.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn file_rows(self) -> Result<Vec<FileRow>, StoreError> {
        let raw = self
            .collect_all(schema::FILE_ROWS, RawFile::read)
            .map_err(sqlite("reading files"))?;
        raw.into_iter().map(RawFile::check).collect()
    }

    /// The path of every stored file, in the order [`RepoPath`] sorts them: by the bytes of the
    /// path.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored paths that cannot be read back.
    pub fn file_paths(self) -> Result<Vec<RepoPath>, StoreError> {
        Ok(self.file_ids()?.into_iter().map(|(path, _)| path).collect())
    }

    /// Every stored file's path with its row, in the order [`RepoPath`] sorts the paths.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored paths that cannot be read back.
    pub fn file_ids(self) -> Result<Vec<(RepoPath, FileId)>, StoreError> {
        let raw: Vec<(i64, String)> = self
            .collect_all(schema::FILE_PATHS, |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(sqlite("reading file paths"))?;
        raw.iter()
            .map(|(id, path)| Ok((values::path(path, "files.path", *id)?, FileId(*id))))
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

    /// Every folder that has a node, with its node, by path.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored paths that cannot be read back.
    pub fn folder_nodes(self) -> Result<Vec<(RepoPath, NodeId)>, StoreError> {
        self.paths_with_nodes(schema::FOLDER_NODES, "nodes.path")
    }

    /// Every stored file, with its node, by path.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored paths that cannot be read back.
    pub fn file_nodes(self) -> Result<Vec<(RepoPath, NodeId)>, StoreError> {
        self.paths_with_nodes(schema::FILE_NODES, "files.path")
    }

    /// How many stored files have a language and none of the flags in `without`, and how many
    /// lines those files hold.
    ///
    /// # Errors
    ///
    /// SQLite errors and counts that cannot be read back.
    pub fn source_size(self, without: FileFlags) -> Result<(u64, u64), StoreError> {
        let (files, lines): (i64, i64) = self
            .connection
            .query_row(schema::SOURCE_SIZE, [without.bits()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(sqlite("counting source files"))?;
        Ok((
            values::number(files, "files", 0)?,
            values::number(lines, "files.loc", 0)?,
        ))
    }

    /// Every symbol's node, by symbol id.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn symbol_nodes(self) -> Result<Vec<(SymbolId, NodeId)>, StoreError> {
        let raw: Vec<(i64, i64)> = self
            .collect_all(schema::SYMBOL_NODES, |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(sqlite("reading symbol nodes"))?;
        Ok(raw
            .into_iter()
            .map(|(symbol, node)| (SymbolId(symbol), NodeId(node)))
            .collect())
    }

    /// The nodes the edges of `edge_type` into `node` start from, in order.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn sources_into(
        self,
        node: NodeId,
        edge_type: EdgeType,
    ) -> Result<Vec<NodeId>, StoreError> {
        let raw: Vec<i64> = self
            .connection
            .prepare_cached(schema::SOURCES_INTO)
            .and_then(|mut statement| {
                statement
                    .query_map((node.get(), edge_type.number()), |row| row.get(0))?
                    .collect()
            })
            .map_err(sqlite("reading edge sources"))?;
        Ok(raw.into_iter().map(NodeId).collect())
    }

    /// Every node an edge of `edge_type` starts from, in order.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn edge_sources(self, edge_type: EdgeType) -> Result<Vec<NodeId>, StoreError> {
        let raw = self
            .collect(schema::EDGE_SOURCES, edge_type.number(), |row| row.get(0))
            .map_err(sqlite("reading edge sources"))?;
        Ok(raw.into_iter().map(NodeId).collect())
    }

    /// Every symbol's name, with the file it is defined in and that file's language, by id.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn symbol_names(self) -> Result<Vec<SymbolName>, StoreError> {
        let raw = self
            .collect_all(schema::SYMBOL_NAMES, RawSymbolName::read)
            .map_err(sqlite("reading symbol names"))?;
        raw.into_iter().map(RawSymbolName::check).collect()
    }

    /// Every reference's name, with the file it is in, that file's language and what the
    /// reference is tied to, by id.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn ref_names(self) -> Result<Vec<RefName>, StoreError> {
        let raw = self
            .collect_all(schema::REF_NAMES, RawRefName::read)
            .map_err(sqlite("reading reference names"))?;
        raw.into_iter().map(RawRefName::check).collect()
    }

    /// The search tokens of `node`, as stored: lower-case words joined by spaces. `None` when the
    /// node has no search row.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn search_tokens(self, node: NodeId) -> Result<Option<String>, StoreError> {
        self.connection
            .prepare_cached(schema::SEARCH_TOKENS)
            .and_then(|mut statement| {
                statement
                    .query_row([node.get()], |row| row.get(0))
                    .optional()
            })
            .map_err(sqlite("reading search tokens"))
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

    /// The paths and nodes `sql` selects, each row as the id of the row that holds the path (which
    /// a corrupt path is reported by), the path, read back from `column`, and the node id. For a
    /// folder the row holding the path is its node, so its node id comes twice.
    fn paths_with_nodes(
        self,
        sql: &'static str,
        column: &'static str,
    ) -> Result<Vec<(RepoPath, NodeId)>, StoreError> {
        let raw: Vec<(i64, String, i64)> = self
            .collect_all(sql, |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(sqlite("reading paths and nodes"))?;
        raw.into_iter()
            .map(|(id, path, node)| Ok((values::path(&path, column, id)?, NodeId(node))))
            .collect()
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

    /// Every row `sql`, which takes no parameter, selects, read by `read`.
    fn collect_all<T>(
        self,
        sql: &'static str,
        read: fn(&Row<'_>) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<Vec<T>> {
        let mut statement = self.connection.prepare_cached(sql)?;
        statement.query_map([], read)?.collect()
    }
}
