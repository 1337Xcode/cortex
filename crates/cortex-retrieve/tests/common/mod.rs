//! What the retrieval tests share: a repository on disk with a store, files written to both as the
//! index would write them, and links between their nodes.

#![allow(
    dead_code,
    reason = "each test file uses the helpers it needs, and the others are unused there"
)]

use cortex_core::{
    Agent, ContentHash, EdgeType, FileFlags, Identifier, InterestTier, Language, RepoId, RepoPath,
    ShortHash, SymbolKind, Ulid,
};
use cortex_store::{
    EdgeRecord, FileIndex, FileRecord, InterestRow, NodeId, Reader, SessionRecord, SymbolRecord,
    TaskRecord, Writer, WriterOptions,
};

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// One definition to write: its name, kind, first and last lines, and the position of the
/// definition around it in the same list, if any.
#[derive(Debug, Clone, Copy)]
pub struct Def<'a> {
    /// Its name; its qualified name is its parents' names and its own joined by `.`.
    pub name: &'a str,
    /// What it is.
    pub kind: SymbolKind,
    /// Its first line.
    pub start: u32,
    /// Its last line.
    pub end: u32,
    /// The position of the definition around it.
    pub parent: Option<usize>,
}

/// A function from line `start` to line `end`, inside nothing.
pub fn function(name: &str, start: u32, end: u32) -> Def<'_> {
    Def {
        name,
        kind: SymbolKind::Function,
        start,
        end,
        parent: None,
    }
}

/// A repository in a temporary folder with its store.
pub struct Repo {
    /// The repository's root folder.
    pub root: tempfile::TempDir,
    /// The store's writer.
    pub writer: Writer,
}

impl Repo {
    /// An empty repository and store.
    pub fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let writer = Writer::open(&WriterOptions {
            folder: &root.path().join(".cortex"),
            repo_id: &RepoId::of(root.path()),
            mmap_bytes: 0,
            now_ms: 1,
        })?;
        Ok(Self { root, writer })
    }

    /// Writes `text` to `path` on disk and stores the file as the index would after reading it,
    /// with the definitions `defs`, flagged `flags`; returns the file's node and then each
    /// definition's node, in the order given. The folder above it gets its node.
    pub fn file(
        &mut self,
        path: &str,
        text: &str,
        defs: &[Def<'_>],
        flags: FileFlags,
    ) -> TestResult<Vec<NodeId>> {
        let path = RepoPath::new(path)?;
        let native = path.to_native(self.root.path());
        if let Some(folder) = native.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&native, text)?;
        let symbols = defs
            .iter()
            .map(|def| {
                let qual_name = qualified(defs, def);
                Ok(SymbolRecord {
                    parent: def.parent,
                    kind: def.kind,
                    name: Identifier::new(def.name)?,
                    qual_name: Identifier::new(&qual_name)?,
                    start_byte: u64::from(def.start),
                    end_byte: u64::from(def.end),
                    start_line: def.start,
                    end_line: def.end,
                    sig_hash: ShortHash::of(def.name.as_bytes()),
                    body_hash: ShortHash::of(qual_name.as_bytes()),
                })
            })
            .collect::<TestResult<Vec<SymbolRecord>>>()?;
        let mut transaction = self.writer.transaction()?;
        transaction.replace_file(&FileIndex {
            file: FileRecord {
                path: path.clone(),
                language: Some(Language::Python),
                size_bytes: u64::try_from(text.len())?,
                line_count: Some(cortex_index::count_lines(text.as_bytes())),
                content_hash: Some(ContentHash::of(text.as_bytes())),
                mtime_ns: 1,
                flags: FileFlags::from_bits(flags.bits() | FileFlags::PARSED.bits()),
                indexed_at_ms: 1,
            },
            symbols,
            refs: Vec::new(),
            passes_on: Vec::new(),
        })?;
        if let Some(folder) = path.parent().filter(|folder| !folder.is_root()) {
            transaction.folder_node(&folder)?;
        }
        let file = transaction
            .queries()
            .file(&path)?
            .ok_or("a stored file")?
            .id;
        let mut nodes = vec![
            transaction
                .queries()
                .file_node_at(&path)?
                .ok_or("a file node")?,
        ];
        let spans = transaction.queries().symbol_spans_in_file(file)?;
        // The spans come in the order the definitions start; the definitions are given that way.
        nodes.extend(spans.into_iter().map(|span| span.node));
        transaction.commit()?;
        Ok(nodes)
    }

    /// Stores a link of `kind` from `src` to `dst`, of weight `weight`, seen `count` times, last at
    /// task `last_seq`, alongside the node's other links.
    pub fn link(
        &mut self,
        (src, dst): (NodeId, NodeId),
        kind: EdgeType,
        (weight, count, last_seq): (f64, u32, i64),
    ) -> TestResult {
        let mut transaction = self.writer.transaction()?;
        transaction.set_edges(
            kind,
            &[(
                src,
                EdgeRecord {
                    dst,
                    weight,
                    count,
                    last_seq,
                    is_directed: kind != EdgeType::CoAccess && kind != EdgeType::Containment,
                },
            )],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Stores each node's interest of `rows`: its score as of a task, and the tier it was given.
    pub fn interest(&mut self, rows: &[(NodeId, (f64, u64), InterestTier)]) -> TestResult {
        let rows: Vec<InterestRow> = rows
            .iter()
            .map(|(node, (score, last_seq), tier)| InterestRow {
                node: *node,
                score: *score,
                last_seq: *last_seq,
                tier: *tier,
            })
            .collect();
        let mut transaction = self.writer.transaction()?;
        transaction.replace_interest(&rows)?;
        transaction.commit()?;
        Ok(())
    }

    /// Starts `count` tasks in a session of their own, so that the latest task is number `count`.
    pub fn tasks(&mut self, count: u32) -> TestResult {
        let session_id = Identifier::new("tasks")?;
        let mut transaction = self.writer.transaction()?;
        transaction.record_session(&SessionRecord {
            session_id: session_id.clone(),
            agent: Agent::Claude,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 1,
        })?;
        for at in 1..=count {
            transaction.start_task(&TaskRecord {
                task_id: Ulid::from_seed(u64::from(at), b"task"),
                session_id: session_id.clone(),
                started_at_ms: i64::from(at),
                prompt_hash: None,
                prompt_excerpt: None,
                holdout: None,
            })?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// A reader of the store.
    pub fn reader(&self) -> TestResult<Reader> {
        Ok(Reader::open(&self.root.path().join(".cortex"), 0)?)
    }
}

/// The qualified name of `def` among `defs`: its parents' names and its own, joined by `.`.
fn qualified(defs: &[Def<'_>], def: &Def<'_>) -> String {
    let mut names = vec![def.name];
    let mut parent = def.parent;
    while let Some(at) = parent {
        let Some(outer) = defs.get(at) else {
            break;
        };
        names.push(outer.name);
        parent = outer.parent;
    }
    names.reverse();
    names.join(".")
}
