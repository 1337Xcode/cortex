//! What the writer is given, and what it reports back.
//!
//! The only text in a record is a [`RepoPath`] or an [`Identifier`]: a path inside the repository
//! or a name of one line and at most 256 bytes. No record has a field for code, comments, prompt
//! text or tool output. The store does not judge whether a name is a name: keeping code out of
//! names is up to whoever builds the records.

use cortex_core::{
    ContentHash, FileFlags, Identifier, Language, RefKind, RepoPath, Resolution, ShortHash,
    SymbolKind,
};

use crate::ids::{FileId, NodeId, RefId, SymbolId};

/// A file as the index records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    /// Where the file is.
    pub path: RepoPath,
    /// Its language, when Cortex knows it.
    pub language: Option<Language>,
    /// Its size.
    pub size_bytes: u64,
    /// How many lines it has, when it was read.
    pub line_count: Option<u64>,
    /// The hash of its contents, when it was read.
    pub content_hash: Option<ContentHash>,
    /// Its modification time in nanoseconds since the Unix epoch, as the file system reports it.
    pub mtime_ns: i64,
    /// What else is known about it.
    pub flags: FileFlags,
    /// When it was indexed, in UTC milliseconds.
    pub indexed_at_ms: i64,
}

/// A definition found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRecord {
    /// The position of the enclosing definition in the same file's list; it comes before this one.
    pub parent: Option<usize>,
    /// What it is.
    pub kind: SymbolKind,
    /// Its name.
    pub name: Identifier,
    /// Its name with the names of the definitions around it, such as `Parser.parse`.
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

/// A use of a name found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefRecord {
    /// The position of the definition the use sits in, in the same file's symbol list.
    pub symbol: Option<usize>,
    /// The name used.
    pub dst_name: Identifier,
    /// How it is used.
    pub kind: RefKind,
    /// The 1-based line of the use.
    pub line: u32,
}

/// Everything the index records about one file: written together, replacing what was there.
// INVARIANT: no-content-stored. The writer takes a file's rows only in this form, which has no
// field for code, comments, prompt text or tool output: its text is paths and one-line names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIndex {
    /// The file.
    pub file: FileRecord,
    /// Its definitions, each after the one that encloses it.
    pub symbols: Vec<SymbolRecord>,
    /// The names it uses.
    pub refs: Vec<RefRecord>,
}

/// What became of one symbol when a file was replaced or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolChange {
    /// The symbol. A removed symbol's id no longer exists.
    pub symbol: SymbolId,
    /// Its name.
    pub name: Identifier,
    /// What happened to it.
    pub kind: ChangeKind,
}

/// What happened to a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// It is new.
    Added,
    /// It is gone.
    Removed,
    /// Its declaration header changed, and so its whole definition.
    Signature,
    /// Its definition changed below an unchanged header.
    Body,
}

/// What the writer did with a [`FileIndex`]. Ids are in the same order as the index's lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// The file's row.
    pub file: FileId,
    /// The file's node.
    pub file_node: NodeId,
    /// Whether the file was not in the index before.
    pub is_new: bool,
    /// The symbols' rows. A symbol that matches one already stored, by qualified name and kind,
    /// keeps its id, and so do its node and everything attached to the node.
    pub symbols: Vec<SymbolId>,
    /// The symbols' nodes.
    pub symbol_nodes: Vec<NodeId>,
    /// The references' rows, all unresolved until [`crate::WriteTx::set_resolutions`].
    pub refs: Vec<RefId>,
    /// Symbols added, changed or removed: the first two in the index's order, then the removed
    /// ones by id.
    pub changes: Vec<SymbolChange>,
}

/// What the writer removed with a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRemoval {
    /// The file's former row.
    pub file: FileId,
    /// Its symbols, each with [`ChangeKind::Removed`], by id.
    pub removed: Vec<SymbolChange>,
}

/// The symbol a reference was tied to, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The symbol.
    pub symbol: SymbolId,
    /// How the reference was tied to it, which sets its confidence.
    pub resolution: Resolution,
}

/// A reference's target, or `None` to mark it unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// The reference.
    pub reference: RefId,
    /// Its target.
    pub target: Option<Target>,
}

/// An edge from a node, as [`crate::WriteTx::replace_edges`] takes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRecord {
    /// The node it points to.
    pub dst: NodeId,
    /// Its weight.
    pub weight: f64,
    /// How many observations support it.
    pub count: u32,
    /// The task sequence number of the last observation, 0 for none.
    pub last_seq: i64,
    /// Whether it points one way only.
    pub is_directed: bool,
}
