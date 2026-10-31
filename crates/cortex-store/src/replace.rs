//! Replacing and removing a file's rows while keeping the ids of the symbols that survive.
//!
//! A stored symbol keeps its row, its node and everything attached to the node for as long as the
//! file still defines a symbol with the same qualified name and kind. When a file defines several
//! with the same qualified name and kind, they are matched in the order they appear. References are
//! written afresh each time: nothing points at a reference.

use std::collections::{BTreeMap, VecDeque};

use cortex_core::{
    FileFlags, Identifier, Language, NodeKind, RepoPath, Resolution, ShortHash, SymbolKind,
};
use rusqlite::{Connection, OptionalExtension};

use crate::error::{StoreError, sqlite};
use crate::ids::{FileId, NodeId, RefId, SymbolId};
use crate::nodes::{remove_node, upsert_node, write_tokens};
use crate::queries::Queries;
use crate::records::{
    ChangeKind, FileChange, FileIndex, FileRecord, FileRemoval, RefRecord, SymbolChange,
    SymbolRecord,
};
use crate::schema;
use crate::values;

/// A stored symbol, as far as matching needs it.
struct StoredSymbol {
    /// Its row.
    id: SymbolId,
    /// What it is.
    kind: SymbolKind,
    /// Its name.
    name: Identifier,
    /// Its qualified name.
    qual_name: Identifier,
    /// The hash of its declaration header.
    sig_hash: ShortHash,
    /// The hash of the whole definition.
    body_hash: ShortHash,
}

/// The symbols written for a file: ids and nodes in the file's order, and what changed.
struct WrittenSymbols {
    /// The symbols' rows.
    ids: Vec<SymbolId>,
    /// The symbols' nodes.
    nodes: Vec<NodeId>,
    /// Symbols added, changed or removed.
    changes: Vec<SymbolChange>,
}

/// Writes `index` over whatever was stored for its file.
pub(crate) fn replace_file(
    connection: &Connection,
    index: &FileIndex,
) -> Result<FileChange, StoreError> {
    let path = &index.file.path;
    let (file, is_new) = upsert_file(connection, &index.file)?;
    let file_node = upsert_node(connection, NodeKind::File, file.get(), path)?;
    write_tokens(connection, file_node, &[path.as_str()])?;
    let written = write_symbols(connection, file, path, &index.symbols)?;
    connection
        .execute(schema::REFS_DELETE_IN_FILE, [file.get()])
        .map_err(sqlite("deleting references"))?;
    let mut refs = Vec::with_capacity(index.refs.len());
    for (position, reference) in index.refs.iter().enumerate() {
        let symbol = symbol_at(&written.ids, reference.symbol, |target| {
            StoreError::Position {
                path: path.clone(),
                what: "reference",
                index: position,
                position: target,
            }
        })?;
        refs.push(insert_ref(connection, file, symbol, reference)?);
    }
    Ok(FileChange {
        file,
        file_node,
        is_new,
        symbols: written.ids,
        symbol_nodes: written.nodes,
        refs,
        changes: written.changes,
    })
}

/// Writes a file's new symbols over its stored ones, matching them by qualified name and kind,
/// and removes the stored symbols nothing matched.
fn write_symbols(
    connection: &Connection,
    file: FileId,
    path: &RepoPath,
    symbols: &[SymbolRecord],
) -> Result<WrittenSymbols, StoreError> {
    let mut stored: BTreeMap<(SymbolKind, Identifier), VecDeque<StoredSymbol>> = BTreeMap::new();
    for symbol in stored_symbols(connection, file)? {
        stored
            .entry((symbol.kind, symbol.qual_name.clone()))
            .or_default()
            .push_back(symbol);
    }
    let mut written = WrittenSymbols {
        ids: Vec::with_capacity(symbols.len()),
        nodes: Vec::with_capacity(symbols.len()),
        changes: Vec::new(),
    };
    for (position, symbol) in symbols.iter().enumerate() {
        // Only the symbols before this one have ids yet, so a parent anywhere else is refused.
        let parent = symbol_at(&written.ids, symbol.parent, |parent| StoreError::Position {
            path: path.clone(),
            what: "symbol",
            index: position,
            position: parent,
        })?;
        let earlier = stored
            .get_mut(&(symbol.kind, symbol.qual_name.clone()))
            .and_then(VecDeque::pop_front);
        let (id, change) = match earlier {
            Some(old) => {
                update_symbol(connection, old.id, parent, symbol)?;
                (old.id, change_between(&old, symbol))
            }
            None => (
                insert_symbol(connection, file, parent, symbol)?,
                Some(ChangeKind::Added),
            ),
        };
        if let Some(kind) = change {
            written.changes.push(SymbolChange {
                symbol: id,
                name: symbol.name.clone(),
                kind,
            });
        }
        let node = upsert_node(connection, NodeKind::Symbol, id.get(), path)?;
        write_tokens(
            connection,
            node,
            &[symbol.name.as_str(), symbol.qual_name.as_str()],
        )?;
        written.nodes.push(node);
        written.ids.push(id);
    }
    let mut gone: Vec<StoredSymbol> = stored.into_values().flatten().collect();
    gone.sort_by_key(|old| old.id);
    for old in gone {
        written.changes.push(remove_symbol(connection, old)?);
    }
    Ok(written)
}

/// The id at `position` among `ids`, or `None` without a position; `refuse` makes the error for
/// a position past the end.
fn symbol_at(
    ids: &[SymbolId],
    position: Option<usize>,
    refuse: impl FnOnce(usize) -> StoreError,
) -> Result<Option<SymbolId>, StoreError> {
    match position {
        None => Ok(None),
        Some(position) => ids
            .get(position)
            .copied()
            .map(Some)
            .ok_or_else(|| refuse(position)),
    }
}

/// Deletes everything stored for the file at `path`; `None` when nothing was.
pub(crate) fn remove_file(
    connection: &Connection,
    path: &RepoPath,
) -> Result<Option<FileRemoval>, StoreError> {
    let Some(file) = file_id(connection, path)? else {
        return Ok(None);
    };
    let mut removed = Vec::new();
    for old in stored_symbols(connection, file)? {
        removed.push(remove_symbol(connection, old)?);
    }
    connection
        .execute(schema::REFS_DELETE_IN_FILE, [file.get()])
        .map_err(sqlite("deleting references"))?;
    if let Some(node) = Queries::new(connection).file_node(file)? {
        remove_node(connection, node)?;
    }
    connection
        .execute(schema::FILE_DELETE, [file.get()])
        .map_err(sqlite("deleting a file"))?;
    Ok(Some(FileRemoval { file, removed }))
}

/// What changed between a stored symbol and the definition that matched it.
fn change_between(old: &StoredSymbol, new: &SymbolRecord) -> Option<ChangeKind> {
    if old.sig_hash != new.sig_hash {
        Some(ChangeKind::Signature)
    } else if old.body_hash != new.body_hash {
        Some(ChangeKind::Body)
    } else {
        None
    }
}

/// The id of the file at `path`.
fn file_id(connection: &Connection, path: &RepoPath) -> Result<Option<FileId>, StoreError> {
    connection
        .prepare_cached(schema::FILE_ID)
        .and_then(|mut statement| {
            statement
                .query_row([path.as_str()], |row| row.get(0))
                .optional()
        })
        .map(|id| id.map(FileId))
        .map_err(sqlite("finding a file"))
}

/// Writes the file's row and returns its id and whether it is new.
fn upsert_file(connection: &Connection, file: &FileRecord) -> Result<(FileId, bool), StoreError> {
    let line_count = file
        .line_count
        .map(|lines| values::integer(lines, "files.loc"))
        .transpose()?;
    let params = (
        file.path.as_str(),
        file.language.map(Language::name),
        values::integer(file.size_bytes, "files.bytes")?,
        line_count,
        file.content_hash.as_ref().map(|hash| *hash.as_bytes()),
        file.mtime_ns,
        FileFlags::bits(file.flags),
        file.indexed_at_ms,
    );
    match file_id(connection, &file.path)? {
        Some(id) => {
            connection
                .prepare_cached(schema::FILE_UPDATE)
                .and_then(|mut statement| statement.execute(params))
                .map_err(sqlite("updating a file"))?;
            Ok((id, false))
        }
        None => connection
            .prepare_cached(schema::FILE_INSERT)
            .and_then(|mut statement| statement.query_row(params, |row| row.get(0)))
            .map(|id| (FileId(id), true))
            .map_err(sqlite("adding a file")),
    }
}

/// The symbols stored for `file`, oldest first.
fn stored_symbols(connection: &Connection, file: FileId) -> Result<Vec<StoredSymbol>, StoreError> {
    type Raw = (i64, String, String, String, [u8; 16], [u8; 16]);
    let raw: Vec<Raw> = connection
        .prepare_cached(schema::SYMBOLS_TO_MATCH)
        .and_then(|mut statement| {
            statement
                .query_map([file.get()], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?
                .collect()
        })
        .map_err(sqlite("reading symbols"))?;
    raw.into_iter()
        .map(|(id, kind, name, qual_name, sig_hash, body_hash)| {
            Ok(StoredSymbol {
                id: SymbolId(id),
                kind: values::kind(&kind, SymbolKind::from_name, "symbols.kind", id)?,
                name: values::identifier(&name, "symbols.name", id)?,
                qual_name: values::identifier(&qual_name, "symbols.qual_name", id)?,
                sig_hash: ShortHash::from_bytes(sig_hash),
                body_hash: ShortHash::from_bytes(body_hash),
            })
        })
        .collect()
}

/// Adds a symbol to `file` and returns its id.
fn insert_symbol(
    connection: &Connection,
    file: FileId,
    parent: Option<SymbolId>,
    symbol: &SymbolRecord,
) -> Result<SymbolId, StoreError> {
    let params = (
        file.get(),
        parent.map(SymbolId::get),
        symbol.kind.name(),
        symbol.name.as_str(),
        symbol.qual_name.as_str(),
        values::integer(symbol.start_byte, "symbols.start_byte")?,
        values::integer(symbol.end_byte, "symbols.end_byte")?,
        symbol.start_line,
        symbol.end_line,
        symbol.sig_hash.as_bytes(),
        symbol.body_hash.as_bytes(),
    );
    connection
        .prepare_cached(schema::SYMBOL_INSERT)
        .and_then(|mut statement| statement.query_row(params, |row| row.get(0)))
        .map(SymbolId)
        .map_err(sqlite("adding a symbol"))
}

/// Rewrites a matched symbol's row with the new definition.
fn update_symbol(
    connection: &Connection,
    id: SymbolId,
    parent: Option<SymbolId>,
    symbol: &SymbolRecord,
) -> Result<(), StoreError> {
    let params = (
        id.get(),
        parent.map(SymbolId::get),
        symbol.name.as_str(),
        values::integer(symbol.start_byte, "symbols.start_byte")?,
        values::integer(symbol.end_byte, "symbols.end_byte")?,
        symbol.start_line,
        symbol.end_line,
        symbol.sig_hash.as_bytes(),
        symbol.body_hash.as_bytes(),
    );
    connection
        .prepare_cached(schema::SYMBOL_UPDATE)
        .and_then(|mut statement| statement.execute(params))
        .map(drop)
        .map_err(sqlite("updating a symbol"))
}

/// Deletes a symbol, its node and the node's edges, and marks the references to it unresolved.
fn remove_symbol(connection: &Connection, old: StoredSymbol) -> Result<SymbolChange, StoreError> {
    connection
        .prepare_cached(schema::REFS_UNRESOLVE_TO)
        .and_then(|mut statement| statement.execute((old.id.get(), Resolution::ImportPath.name())))
        .map_err(sqlite("unresolving references"))?;
    if let Some(node) = Queries::new(connection).symbol_node(old.id)? {
        remove_node(connection, node)?;
    }
    connection
        .prepare_cached(schema::SYMBOL_DELETE)
        .and_then(|mut statement| statement.execute([old.id.get()]))
        .map_err(sqlite("deleting a symbol"))?;
    Ok(SymbolChange {
        symbol: old.id,
        name: old.name,
        kind: ChangeKind::Removed,
    })
}

/// Adds an unresolved reference from `file`, used inside `symbol`, and returns its id.
fn insert_ref(
    connection: &Connection,
    file: FileId,
    symbol: Option<SymbolId>,
    reference: &RefRecord,
) -> Result<RefId, StoreError> {
    let params = (
        file.get(),
        symbol.map(SymbolId::get),
        reference.dst_name.as_str(),
        reference.kind.name(),
        reference.line,
    );
    connection
        .prepare_cached(schema::REF_INSERT)
        .and_then(|mut statement| statement.query_row(params, |row| row.get(0)))
        .map(RefId)
        .map_err(sqlite("adding a reference"))
}
