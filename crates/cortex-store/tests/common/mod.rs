//! Builders shared by the store's integration tests. Each test file uses part of them.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these builders"
)]

use std::error::Error;
use std::path::PathBuf;

use cortex_core::{
    ContentHash, FileFlags, Identifier, Language, RefKind, RepoId, RepoPath, ShortHash, SymbolKind,
};
use cortex_store::{
    FileIndex, FileRecord, Reader, RefRecord, StoreError, SymbolRecord, Writer, WriterOptions,
};
use tempfile::TempDir;

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// The default memory map, 256 MiB.
pub const MMAP_BYTES: u64 = 268_435_456;

/// A temporary repository root and the `.cortex` folder inside it, which does not exist yet.
pub struct Scratch {
    /// The repository root, deleted when the test ends.
    pub root: TempDir,
    /// `<root>/.cortex`.
    pub folder: PathBuf,
    /// The repository's id.
    pub repo_id: RepoId,
}

impl Scratch {
    /// A new empty repository.
    pub fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let folder = root.path().join(".cortex");
        let repo_id = RepoId::of(root.path());
        Ok(Self {
            root,
            folder,
            repo_id,
        })
    }

    /// Opens the writer at time `now_ms`.
    pub fn writer(&self, now_ms: i64) -> Result<Writer, StoreError> {
        Writer::open(&WriterOptions {
            folder: &self.folder,
            repo_id: &self.repo_id,
            mmap_bytes: MMAP_BYTES,
            now_ms,
        })
    }

    /// Opens a reader.
    pub fn reader(&self) -> Result<Reader, StoreError> {
        Reader::open(&self.folder, MMAP_BYTES)
    }
}

/// A name.
pub fn name(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
}

/// A repository path.
pub fn path(text: &str) -> TestResult<RepoPath> {
    Ok(RepoPath::new(text)?)
}

/// A parsed Python file at `path`, with fixed sizes and times.
pub fn file(path_text: &str) -> TestResult<FileRecord> {
    Ok(FileRecord {
        path: path(path_text)?,
        language: Some(Language::Python),
        size_bytes: 120,
        line_count: Some(12),
        content_hash: Some(ContentHash::of(path_text.as_bytes())),
        mtime_ns: 1_700_000_000_000_000_000,
        flags: FileFlags::empty().with(FileFlags::PARSED),
        indexed_at_ms: 5,
    })
}

/// A definition of `kind` called `qual_name`, named by its last dotted part, inside the symbol at
/// `parent`. Its hashes are those of the `sig` and `body` labels, so equal labels mean an
/// unchanged header or definition.
pub fn symbol(
    kind: SymbolKind,
    qual_name: &str,
    parent: Option<usize>,
    (sig, body): (&str, &str),
) -> TestResult<SymbolRecord> {
    let short = qual_name.rsplit('.').next().unwrap_or(qual_name);
    Ok(SymbolRecord {
        parent,
        kind,
        name: name(short)?,
        qual_name: name(qual_name)?,
        start_byte: 0,
        end_byte: 10,
        start_line: 1,
        end_line: 2,
        sig_hash: ShortHash::of(sig.as_bytes()),
        body_hash: ShortHash::of(body.as_bytes()),
    })
}

/// A use of `dst_name` on `line`, inside the symbol at `symbol`.
pub fn reference(
    symbol: Option<usize>,
    dst_name: &str,
    kind: RefKind,
    line: u32,
) -> TestResult<RefRecord> {
    Ok(RefRecord {
        symbol,
        dst_name: name(dst_name)?,
        kind,
        line,
    })
}

/// The index of the Python file at `path_text` with `symbols` and `refs`.
pub fn index(
    path_text: &str,
    symbols: Vec<SymbolRecord>,
    refs: Vec<RefRecord>,
) -> TestResult<FileIndex> {
    Ok(FileIndex {
        file: file(path_text)?,
        symbols,
        refs,
        passes_on: Vec::new(),
    })
}
