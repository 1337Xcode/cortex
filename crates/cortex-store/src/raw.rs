//! Rows as SQLite returns them, and the checks that turn them into typed rows.
//!
//! A query reads the columns it needs into plain values first and checks them second, so a
//! damaged row is reported as [`StoreError::Corrupt`] with its table, column and id.

use cortex_core::{ContentHash, EdgeType, FileFlags, RefKind, Resolution, ShortHash, SymbolKind};
use rusqlite::Row;

use crate::error::StoreError;
use crate::ids::{FileId, NodeId, RefId, SymbolId};
use crate::records::{EdgeRecord, FileRecord, Target};
use crate::rows::{EdgeRow, FileRow, RefName, RefRow, SymbolName, SymbolRow};
use crate::values;

/// A `files` row as SQLite returns it.
pub(crate) struct RawFile {
    /// `file_id`.
    id: i64,
    /// `path`.
    path: String,
    /// `lang`.
    lang: Option<String>,
    /// `bytes`.
    bytes: i64,
    /// `loc`.
    loc: Option<i64>,
    /// `content_hash`.
    content_hash: Option<[u8; 32]>,
    /// `mtime_ns`.
    mtime_ns: i64,
    /// `flags`.
    flags: i64,
    /// `indexed_at`.
    indexed_at: i64,
}

impl RawFile {
    /// Reads the columns of [`crate::schema::FILE_BY_PATH`].
    pub(crate) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            path: row.get(1)?,
            lang: row.get(2)?,
            bytes: row.get(3)?,
            loc: row.get(4)?,
            content_hash: row.get(5)?,
            mtime_ns: row.get(6)?,
            flags: row.get(7)?,
            indexed_at: row.get(8)?,
        })
    }

    /// The row with every value checked.
    pub(crate) fn check(self) -> Result<FileRow, StoreError> {
        let row = self.id;
        let language = values::language(self.lang, row)?;
        let line_count = self
            .loc
            .map(|lines| values::number(lines, "files.loc", row))
            .transpose()?;
        Ok(FileRow {
            id: FileId(row),
            file: FileRecord {
                path: values::path(&self.path, "files.path", row)?,
                language,
                size_bytes: values::number(self.bytes, "files.bytes", row)?,
                line_count,
                content_hash: self.content_hash.map(ContentHash::from_bytes),
                mtime_ns: self.mtime_ns,
                flags: FileFlags::from_bits(values::number(self.flags, "files.flags", row)?),
                indexed_at_ms: self.indexed_at,
            },
        })
    }
}

/// A `symbols` row as SQLite returns it.
pub(crate) struct RawSymbol {
    /// `symbol_id`.
    id: i64,
    /// `file_id`.
    file: i64,
    /// `parent_id`.
    parent: Option<i64>,
    /// `kind`.
    kind: String,
    /// `name`.
    name: String,
    /// `qual_name`.
    qual_name: String,
    /// `start_byte`, `end_byte`, `start_line` and `end_line`.
    span: [i64; 4],
    /// `sig_hash`.
    sig_hash: [u8; 16],
    /// `body_hash`.
    body_hash: [u8; 16],
}

impl RawSymbol {
    /// Reads the columns of the symbol queries in [`crate::schema`].
    pub(crate) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            file: row.get(1)?,
            parent: row.get(2)?,
            kind: row.get(3)?,
            name: row.get(4)?,
            qual_name: row.get(5)?,
            span: [row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?],
            sig_hash: row.get(10)?,
            body_hash: row.get(11)?,
        })
    }

    /// The row with every value checked.
    pub(crate) fn check(self) -> Result<SymbolRow, StoreError> {
        let row = self.id;
        let [start_byte, end_byte, start_line, end_line] = self.span;
        Ok(SymbolRow {
            id: SymbolId(row),
            file: FileId(self.file),
            parent: self.parent.map(SymbolId),
            kind: values::kind(&self.kind, SymbolKind::from_name, "symbols.kind", row)?,
            name: values::identifier(&self.name, "symbols.name", row)?,
            qual_name: values::identifier(&self.qual_name, "symbols.qual_name", row)?,
            start_byte: values::number(start_byte, "symbols.start_byte", row)?,
            end_byte: values::number(end_byte, "symbols.end_byte", row)?,
            start_line: values::number(start_line, "symbols.start_line", row)?,
            end_line: values::number(end_line, "symbols.end_line", row)?,
            sig_hash: ShortHash::from_bytes(self.sig_hash),
            body_hash: ShortHash::from_bytes(self.body_hash),
        })
    }
}

/// A `refs` row as SQLite returns it.
pub(crate) struct RawRef {
    /// `ref_id`.
    id: i64,
    /// `src_file_id`.
    file: i64,
    /// `src_symbol_id`.
    symbol: Option<i64>,
    /// `dst_name`.
    dst_name: String,
    /// `dst_symbol_id`.
    dst_symbol: Option<i64>,
    /// `kind`.
    kind: String,
    /// `line`.
    line: i64,
    /// `resolution`.
    resolution: Option<String>,
    /// `confidence`.
    confidence: f64,
}

impl RawRef {
    /// Reads the columns of [`crate::schema::REFS_IN_FILE`].
    pub(crate) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            file: row.get(1)?,
            symbol: row.get(2)?,
            dst_name: row.get(3)?,
            dst_symbol: row.get(4)?,
            kind: row.get(5)?,
            line: row.get(6)?,
            resolution: row.get(7)?,
            confidence: row.get(8)?,
        })
    }

    /// The row with every value checked. A target comes with its resolution; a resolution comes
    /// alone only as `import_path`.
    pub(crate) fn check(self) -> Result<RefRow, StoreError> {
        let row = self.id;
        let (target, goes_through_import) = target(self.dst_symbol, self.resolution, row)?;
        Ok(RefRow {
            id: RefId(row),
            file: FileId(self.file),
            symbol: self.symbol.map(SymbolId),
            dst_name: values::identifier(&self.dst_name, "refs.dst_name", row)?,
            kind: values::kind(&self.kind, RefKind::from_name, "refs.kind", row)?,
            line: values::number(self.line, "refs.line", row)?,
            target,
            goes_through_import,
            confidence: checked_confidence(self.confidence, target, row)?,
        })
    }
}

/// The target of the reference in `row`, from its stored symbol and resolution, and whether the
/// reference goes through an import. A symbol comes with its resolution; a resolution without a
/// symbol is `import_path` only, for a reference whose import settles nothing.
fn target(
    symbol: Option<i64>,
    resolution: Option<String>,
    row: i64,
) -> Result<(Option<Target>, bool), StoreError> {
    let resolution = resolution
        .map(|name| values::kind(&name, Resolution::from_name, "refs.resolution", row))
        .transpose()?;
    match (symbol, resolution) {
        (Some(symbol), Some(resolution)) => {
            let target = Target {
                symbol: SymbolId(symbol),
                resolution,
            };
            Ok((Some(target), resolution == Resolution::ImportPath))
        }
        (None, Some(Resolution::ImportPath)) => Ok((None, true)),
        (None, None) => Ok((None, false)),
        _ => Err(StoreError::Corrupt {
            column: "refs.resolution",
            row,
        }),
    }
}

/// The stored confidence of the reference in `row`, which must be its target's resolution's, or 0
/// without a target.
fn checked_confidence(
    confidence: f64,
    target: Option<Target>,
    row: i64,
) -> Result<f64, StoreError> {
    let expected = target.map_or(0.0, |target| target.resolution.confidence());
    if confidence.total_cmp(&expected).is_eq() {
        Ok(confidence)
    } else {
        Err(StoreError::Corrupt {
            column: "refs.confidence",
            row,
        })
    }
}

/// A symbol's name and file as [`crate::schema::SYMBOL_NAMES`] returns them.
pub(crate) struct RawSymbolName {
    /// `symbol_id`.
    id: i64,
    /// `file_id`.
    file: i64,
    /// The file's `lang`.
    lang: Option<String>,
    /// `name`.
    name: String,
    /// Whether `parent_id` is null and `qual_name` is `name`.
    is_top_level: bool,
    /// `kind`.
    kind: String,
}

impl RawSymbolName {
    /// Reads the columns of [`crate::schema::SYMBOL_NAMES`].
    pub(crate) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            file: row.get(1)?,
            lang: row.get(2)?,
            name: row.get(3)?,
            is_top_level: row.get(4)?,
            kind: row.get(5)?,
        })
    }

    /// The row with every value checked.
    pub(crate) fn check(self) -> Result<SymbolName, StoreError> {
        Ok(SymbolName {
            id: SymbolId(self.id),
            file: FileId(self.file),
            language: values::language(self.lang, self.file)?,
            name: values::identifier(&self.name, "symbols.name", self.id)?,
            is_top_level: self.is_top_level,
            kind: values::kind(&self.kind, SymbolKind::from_name, "symbols.kind", self.id)?,
        })
    }
}

/// A reference's name, file and target as [`crate::schema::REF_NAMES`] returns them.
pub(crate) struct RawRefName {
    /// `ref_id`.
    id: i64,
    /// `src_file_id`.
    file: i64,
    /// The file's `lang`.
    lang: Option<String>,
    /// `dst_name`.
    dst_name: String,
    /// `kind`.
    kind: String,
    /// `line`.
    line: i64,
    /// `dst_symbol_id`.
    dst_symbol: Option<i64>,
    /// `resolution`.
    resolution: Option<String>,
    /// `confidence`.
    confidence: f64,
    /// `src_symbol_id`.
    src_symbol: Option<i64>,
}

impl RawRefName {
    /// Reads the columns of [`crate::schema::REF_NAMES`].
    pub(crate) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            file: row.get(1)?,
            lang: row.get(2)?,
            dst_name: row.get(3)?,
            kind: row.get(4)?,
            line: row.get(5)?,
            dst_symbol: row.get(6)?,
            resolution: row.get(7)?,
            confidence: row.get(8)?,
            src_symbol: row.get(9)?,
        })
    }

    /// The row with every value checked, the stored confidence against the target's resolution.
    pub(crate) fn check(self) -> Result<RefName, StoreError> {
        let row = self.id;
        let (target, goes_through_import) = target(self.dst_symbol, self.resolution, row)?;
        checked_confidence(self.confidence, target, row)?;
        Ok(RefName {
            id: RefId(row),
            file: FileId(self.file),
            language: values::language(self.lang, self.file)?,
            name: values::identifier(&self.dst_name, "refs.dst_name", row)?,
            kind: values::kind(&self.kind, RefKind::from_name, "refs.kind", row)?,
            line: values::number(self.line, "refs.line", row)?,
            symbol: self.src_symbol.map(SymbolId),
            target,
            goes_through_import,
        })
    }
}

/// An `edges` row as SQLite returns it.
pub(crate) struct RawEdge {
    /// `src`.
    src: i64,
    /// `dst`.
    dst: i64,
    /// `type`.
    edge_type: i64,
    /// `w`.
    weight: f64,
    /// `n`.
    count: i64,
    /// `last_seq`.
    last_seq: i64,
    /// `directed`.
    directed: i64,
}

impl RawEdge {
    /// Reads the columns of [`crate::schema::EDGES_FROM`].
    pub(crate) fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            src: row.get(0)?,
            dst: row.get(1)?,
            edge_type: row.get(2)?,
            weight: row.get(3)?,
            count: row.get(4)?,
            last_seq: row.get(5)?,
            directed: row.get(6)?,
        })
    }

    /// The row with every value checked. Edges have no id of their own, so errors name the
    /// source node.
    pub(crate) fn check(self) -> Result<EdgeRow, StoreError> {
        let row = self.src;
        let number: u8 = values::number(self.edge_type, "edges.type", row)?;
        let edge_type = EdgeType::from_number(number).ok_or(StoreError::Corrupt {
            column: "edges.type",
            row,
        })?;
        let is_directed = match self.directed {
            0 => false,
            1 => true,
            _ => {
                return Err(StoreError::Corrupt {
                    column: "edges.directed",
                    row,
                });
            }
        };
        Ok(EdgeRow {
            src: NodeId(self.src),
            edge_type,
            edge: EdgeRecord {
                dst: NodeId(self.dst),
                weight: self.weight,
                count: values::number(self.count, "edges.n", row)?,
                last_seq: self.last_seq,
                is_directed,
            },
        })
    }
}
