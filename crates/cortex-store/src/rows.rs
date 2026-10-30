//! What the read queries return.

use cortex_core::{EdgeType, Identifier, Language, RefKind, ShortHash, SymbolKind};

use crate::ids::{FileId, NodeId, RefId, SymbolId};
use crate::records::{EdgeRecord, FileRecord, Target};

/// A stored file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    /// Its row.
    pub id: FileId,
    /// What is recorded about it.
    pub file: FileRecord,
}

/// A stored symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRow {
    /// Its row.
    pub id: SymbolId,
    /// The file it is defined in.
    pub file: FileId,
    /// The definition around it.
    pub parent: Option<SymbolId>,
    /// What it is.
    pub kind: SymbolKind,
    /// Its name.
    pub name: Identifier,
    /// Its qualified name.
    pub qual_name: Identifier,
    /// The byte offset where it starts.
    pub start_byte: u64,
    /// The byte offset just past its end.
    pub end_byte: u64,
    /// The 1-based line where it starts.
    pub start_line: u32,
    /// The 1-based line where it ends.
    pub end_line: u32,
    /// The hash of its declaration header.
    pub sig_hash: ShortHash,
    /// The hash of the whole definition.
    pub body_hash: ShortHash,
}

/// A stored reference.
#[derive(Debug, Clone, PartialEq)]
pub struct RefRow {
    /// Its row.
    pub id: RefId,
    /// The file it is in.
    pub file: FileId,
    /// The definition it sits in.
    pub symbol: Option<SymbolId>,
    /// The name used.
    pub dst_name: Identifier,
    /// How it is used.
    pub kind: RefKind,
    /// The 1-based line of the use.
    pub line: u32,
    /// The symbol it was tied to, if any.
    pub target: Option<Target>,
    /// The stored confidence: the resolution's, or 0 when unresolved.
    pub confidence: f64,
}

/// A stored edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRow {
    /// The node it starts from.
    pub src: NodeId,
    /// Its type.
    pub edge_type: EdgeType,
    /// Where it points and what it weighs.
    pub edge: EdgeRecord,
}

/// How many rows the index holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexCounts {
    /// Files.
    pub files: u64,
    /// Symbols.
    pub symbols: u64,
    /// References.
    pub refs: u64,
    /// References tied to a symbol.
    pub resolved_refs: u64,
    /// Graph nodes.
    pub nodes: u64,
    /// Graph edges.
    pub edges: u64,
}

/// A stored symbol's name and the file it is defined in, as resolution reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolName {
    /// Its row.
    pub id: SymbolId,
    /// The file it is defined in.
    pub file: FileId,
    /// That file's language.
    pub language: Option<Language>,
    /// Its name.
    pub name: Identifier,
}

/// A stored reference's name, the file it is in and what it is tied to, as resolution reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefName {
    /// Its row.
    pub id: RefId,
    /// The file it is in.
    pub file: FileId,
    /// That file's language.
    pub language: Option<Language>,
    /// The name used.
    pub name: Identifier,
    /// The symbol it is tied to, if any.
    pub target: Option<Target>,
}
