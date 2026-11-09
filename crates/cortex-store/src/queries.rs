//! Read queries, shared by readers and by the writer's transactions.

use std::collections::BTreeMap;

use cortex_core::{
    EdgeType, FileFlags, Identifier, Language, NodeKind, RefKind, RepoPath, Resolution, SymbolKind,
};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::error::{StoreError, sqlite};
use crate::ids::{FileId, NodeId, RefId, SymbolId};
use crate::meta::MetaKey;
use crate::passes_on;
use crate::raw::{RawEdge, RawFile, RawRef, RawRefName, RawSymbol, RawSymbolName};
use crate::records::PassedOn;
use crate::rows::{
    EdgeRow, FileRow, IndexCounts, RefName, RefRow, SymbolName, SymbolRow, SymbolSpan,
};
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

    /// The connection the queries run on, for the query modules of other tables.
    pub(crate) fn connection(self) -> &'c Connection {
        self.connection
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

    /// The references tied to `symbol`, wherever they are, by file and then line.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn refs_tied_to(self, symbol: SymbolId) -> Result<Vec<RefRow>, StoreError> {
        let raw = self
            .collect(schema::REFS_TIED_TO, symbol.get(), RawRef::read)
            .map_err(sqlite("reading the references to a symbol"))?;
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

    /// The files that hold a test and have a structural edge into `node`, such as tests that
    /// import it or are named for it, each with the edge's weight, by node.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn test_files_into(self, node: NodeId) -> Result<Vec<(NodeId, f64)>, StoreError> {
        let raw: Vec<(i64, f64)> = self
            .connection
            .prepare_cached(schema::TEST_FILES_INTO)
            .and_then(|mut statement| {
                statement
                    .query_map((node.get(), EdgeType::Structural.number()), |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?
                    .collect()
            })
            .map_err(sqlite("reading the tests of a file"))?;
        Ok(raw
            .into_iter()
            .map(|(src, weight)| (NodeId(src), weight))
            .collect())
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

    /// The symbols in `files`, as [`Queries::symbol_names`] gives them, by id.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn symbol_names_in_files(self, files: &[FileId]) -> Result<Vec<SymbolName>, StoreError> {
        let ids = json_array(files.iter().map(|file| file.get().to_string()));
        let raw = self
            .collect(schema::SYMBOL_NAMES_IN_FILES, ids, RawSymbolName::read)
            .map_err(sqlite("reading symbol names"))?;
        raw.into_iter().map(RawSymbolName::check).collect()
    }

    /// The first `limit` symbols named `name` by id, as [`Queries::symbol_names`] gives them, in
    /// files of `languages`: enough to tell one definition from several.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn symbol_names_named_in(
        self,
        name: &Identifier,
        languages: &[Language],
        limit: u32,
    ) -> Result<Vec<SymbolName>, StoreError> {
        // Language names are fixed lower-case words: quoted, they need no escaping.
        let names = json_array(
            languages
                .iter()
                .map(|language| format!("\"{}\"", language.name())),
        );
        let raw: Vec<RawSymbolName> = self
            .connection
            .prepare_cached(schema::SYMBOL_NAMES_NAMED_IN_LANGUAGES)
            .and_then(|mut statement| {
                statement
                    .query_map((name.as_str(), names, limit), RawSymbolName::read)?
                    .collect()
            })
            .map_err(sqlite("reading symbol names"))?;
        raw.into_iter().map(RawSymbolName::check).collect()
    }

    /// The files holding a reference named `name` that goes through an import, tied through one
    /// or left untied by one. An import's own reference counts, unless the SCIP index tied it.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn files_with_refs_through_imports(
        self,
        name: &Identifier,
    ) -> Result<Vec<FileId>, StoreError> {
        self.collect(
            schema::FILES_WITH_REFS_THROUGH_IMPORTS,
            name.as_str(),
            |row| row.get(0).map(FileId),
        )
        .map_err(sqlite("reading files with references through imports"))
    }

    /// The references named `name` that go by their name, as [`Queries::ref_names`] gives them,
    /// by id: none that goes through an import or is an import's own.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn ref_names_by_name(self, name: &Identifier) -> Result<Vec<RefName>, StoreError> {
        let params = (
            name.as_str(),
            Resolution::ImportPath.name(),
            RefKind::Import.name(),
        );
        let raw: Vec<RawRefName> = self
            .connection
            .prepare_cached(schema::REF_NAMES_BY_NAME)
            .and_then(|mut statement| statement.query_map(params, RawRefName::read)?.collect())
            .map_err(sqlite("reading reference names"))?;
        raw.into_iter().map(RawRefName::check).collect()
    }

    /// The files holding a reference the SCIP index tied to a definition in one of `files`.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn files_with_scip_ties_into(self, files: &[FileId]) -> Result<Vec<FileId>, StoreError> {
        let ids = json_array(files.iter().map(|file| file.get().to_string()));
        self.connection
            .prepare_cached(schema::FILES_TIED_INTO)
            .and_then(|mut statement| {
                statement
                    .query_map((ids, Resolution::Scip.name()), |row| row.get(0).map(FileId))?
                    .collect()
            })
            .map_err(sqlite("reading files with SCIP ties"))
    }

    /// Every reference the SCIP index tied, with its file, by id.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn scip_ties(self) -> Result<Vec<(RefId, FileId)>, StoreError> {
        self.collect(schema::REFS_TIED_AS, Resolution::Scip.name(), |row| {
            Ok((RefId(row.get(0)?), FileId(row.get(1)?)))
        })
        .map_err(sqlite("reading SCIP ties"))
    }

    /// The references in `file`, as [`Queries::ref_names`] gives them, in the order they were
    /// written.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn ref_names_in_file(self, file: FileId) -> Result<Vec<RefName>, StoreError> {
        let raw = self
            .collect(schema::REF_NAMES_IN_FILE, file.get(), RawRefName::read)
            .map_err(sqlite("reading reference names"))?;
        raw.into_iter().map(RawRefName::check).collect()
    }

    /// What the file `file` passes on of what its top-level imports bring in.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn passes_on(self, file: FileId) -> Result<Vec<PassedOn>, StoreError> {
        passes_on::of_file(self.connection, file)
    }

    /// What every file that passes anything on passes on, by file.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn every_passes_on(self) -> Result<BTreeMap<FileId, Vec<PassedOn>>, StoreError> {
        passes_on::of_every_file(self.connection)
    }

    /// The names files pass `taken` on as, other than `taken` itself, in name order: an import
    /// of one of them may find a definition named `taken`.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored names that cannot be read back.
    pub fn names_passed_on_as(self, taken: &Identifier) -> Result<Vec<Identifier>, StoreError> {
        passes_on::aliases(self.connection, taken)
    }

    /// The node of the file at `path`.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn file_node_at(self, path: &RepoPath) -> Result<Option<NodeId>, StoreError> {
        self.connection
            .prepare_cached(schema::FILE_NODE_AT)
            .and_then(|mut statement| {
                statement
                    .query_row([path.as_str()], |row| row.get(0))
                    .optional()
            })
            .map(|node| node.map(NodeId))
            .map_err(sqlite("reading a file's node"))
    }

    /// The node of the folder at `path`, when the folder has one. A folder's node is added when
    /// something first needs it, so a folder the graph has no use for yet has none.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn folder_node_at(self, path: &RepoPath) -> Result<Option<NodeId>, StoreError> {
        self.connection
            .prepare_cached(schema::FOLDER_NODE)
            .and_then(|mut statement| {
                statement
                    .query_row([path.as_str()], |row| row.get(0))
                    .optional()
            })
            .map(|node| node.map(NodeId))
            .map_err(sqlite("reading a folder's node"))
    }

    /// The node, lines, row, enclosing symbol, kind and name of each symbol defined in `file`, in
    /// the order they appear.
    ///
    /// # Errors
    ///
    /// SQLite errors and a stored kind or name that cannot be read back.
    pub fn symbol_spans_in_file(self, file: FileId) -> Result<Vec<SymbolSpan>, StoreError> {
        let mut statement = self
            .connection
            .prepare_cached(schema::SYMBOL_SPANS_IN_FILE)
            .map_err(sqlite("reading a file's symbol spans"))?;
        let rows: Vec<RawSpan> = statement
            .query_map([file.0], |row| {
                Ok(RawSpan {
                    node: row.get(0)?,
                    start_line: row.get(1)?,
                    end_line: row.get(2)?,
                    symbol: row.get(3)?,
                    parent: row.get(4)?,
                    kind: row.get(5)?,
                    name: row.get(6)?,
                })
            })
            .and_then(Iterator::collect)
            .map_err(sqlite("reading a file's symbol spans"))?;
        rows.into_iter().map(RawSpan::check).collect()
    }

    /// The files the file nodes `nodes` stand for, each with its node, by node; a node that stands
    /// for no file is left out.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn files_of_nodes(self, nodes: &[NodeId]) -> Result<Vec<(NodeId, FileRow)>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        let raw: Vec<(i64, RawFile)> = self
            .connection
            .prepare_cached(schema::FILES_OF_NODES)
            .and_then(|mut statement| {
                statement
                    .query_map((listed,), |row| Ok((row.get(9)?, RawFile::read(row)?)))?
                    .collect::<rusqlite::Result<_>>()
            })
            .map_err(sqlite("reading the files of nodes"))?;
        raw.into_iter()
            .map(|(node, file)| Ok((NodeId(node), file.check()?)))
            .collect()
    }

    /// The row and path of the file `node` stands for, when it stands for a file.
    ///
    /// # Errors
    ///
    /// SQLite errors and a stored path that cannot be read back.
    pub fn node_file(self, node: NodeId) -> Result<Option<(FileId, RepoPath)>, StoreError> {
        self.file_by(schema::NODE_FILE, node.0)
    }

    /// The path of the file with the row `file`.
    ///
    /// # Errors
    ///
    /// SQLite errors and a stored path that cannot be read back.
    pub fn file_path(self, file: FileId) -> Result<Option<RepoPath>, StoreError> {
        Ok(self
            .file_by(schema::FILE_PATH_OF, file.get())?
            .map(|(_, path)| path))
    }

    /// The row and path of the file `sql` selects with `key`.
    fn file_by(
        self,
        sql: &'static str,
        key: i64,
    ) -> Result<Option<(FileId, RepoPath)>, StoreError> {
        let raw: Option<(i64, String)> = self
            .connection
            .prepare_cached(sql)
            .and_then(|mut statement| {
                statement
                    .query_row([key], |row| Ok((row.get(0)?, row.get(1)?)))
                    .optional()
            })
            .map_err(sqlite("reading a file's path"))?;
        raw.map(|(id, path)| Ok((FileId(id), values::path(&path, "files.path", id)?)))
            .transpose()
    }

    /// The paths of the files stored below the folder `folder`, by path; `None` when there are
    /// more than `limit`.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored paths that cannot be read back.
    pub fn file_paths_under(
        self,
        folder: &RepoPath,
        limit: usize,
    ) -> Result<Option<Vec<RepoPath>>, StoreError> {
        // Paths below `a/b` sort from `a/b/` up to `a/b0`, `0` following `/`.
        let low = format!("{folder}/");
        let high = format!("{folder}0");
        // One more than the limit is asked for, to tell a full list from a cut one.
        let asked = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let raw: Vec<(i64, String)> = self
            .connection
            .prepare_cached(schema::FILE_PATHS_BETWEEN)
            .and_then(|mut statement| {
                statement
                    .query_map((low, high, asked), |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .map_err(sqlite("listing the files below a folder"))?;
        if raw.len() > limit {
            return Ok(None);
        }
        raw.iter()
            .map(|(id, path)| values::path(path, "files.path", *id))
            .collect::<Result<_, _>>()
            .map(Some)
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

    /// The search tokens of each of `nodes` that has a search row, by node, as
    /// [`Queries::search_tokens`] gives them.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn search_tokens_of(self, nodes: &[NodeId]) -> Result<Vec<(NodeId, String)>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        self.connection
            .prepare_cached(schema::SEARCH_TOKENS_OF)
            .and_then(|mut statement| {
                statement
                    .query_map([listed], |row| Ok((NodeId(row.get(0)?), row.get(1)?)))?
                    .collect()
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

    /// Every edge of `edge_type` that runs from one of `nodes` to another of them, by source and
    /// then target. An undirected edge is stored once, from the smaller node, so a pair shows up
    /// once however the two are given.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn edges_among(
        self,
        nodes: &[NodeId],
        edge_type: EdgeType,
    ) -> Result<Vec<EdgeRow>, StoreError> {
        let listed = json_array(nodes.iter().map(|node| node.get().to_string()));
        let mut statement = self
            .connection
            .prepare_cached(schema::EDGES_AMONG)
            .map_err(sqlite("reading edges"))?;
        let raw = statement
            .query_map((edge_type.number(), listed), RawEdge::read)
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
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
    pub(crate) fn symbols(
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

/// One row of [`schema::SYMBOL_SPANS_IN_FILE`] as SQLite gives it.
struct RawSpan {
    /// `nodes.node_id`.
    node: i64,
    /// `symbols.start_line`.
    start_line: u32,
    /// `symbols.end_line`.
    end_line: u32,
    /// `symbols.symbol_id`.
    symbol: i64,
    /// `symbols.parent_id`.
    parent: Option<i64>,
    /// `symbols.kind`.
    kind: String,
    /// `symbols.name`.
    name: String,
}

impl RawSpan {
    /// The span these values hold, or the error that one cannot be read back.
    fn check(self) -> Result<SymbolSpan, StoreError> {
        Ok(SymbolSpan {
            node: NodeId(self.node),
            start_line: self.start_line,
            end_line: self.end_line,
            symbol: SymbolId(self.symbol),
            parent: self.parent.map(SymbolId),
            kind: values::kind(
                &self.kind,
                SymbolKind::from_name,
                "symbols.kind",
                self.symbol,
            )?,
            name: values::identifier(&self.name, "symbols.name", self.symbol)?,
        })
    }
}

/// `values`, each already written as JSON, as a JSON array: the form `json_each` reads a list of
/// bound values from.
pub(crate) fn json_array(values: impl Iterator<Item = String>) -> String {
    format!("[{}]", values.collect::<Vec<_>>().join(","))
}
