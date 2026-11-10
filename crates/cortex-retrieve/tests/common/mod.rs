//! What the retrieval tests share: a repository on disk with a store, files written to both as the
//! index would write them, and links between their nodes.

#![allow(
    dead_code,
    reason = "each test file uses the helpers it needs, and the others are unused there"
)]

pub mod task;

use cortex_core::{
    Agent, ContentHash, EdgeType, Event, FileFlags, GateFeatures, HookKind, Identifier,
    InterestTier, Language, LineRange, RepoId, RepoPath, ScaleMode, ShortHash, SymbolKind,
    ToolKind, ToolResult, ToolTarget, Ulid,
};
use cortex_learn::{Commands, Recorder};
use cortex_store::{
    ArchetypeRecord, EdgeRecord, FileIndex, FileRecord, InterestRow, NodeId, Reader, SessionRecord,
    SymbolRecord, TaskRecord, Writer, WriterOptions,
};

/// What the gate weighed for a work order these tests record: every prompt's decision is stored
/// with its features, a work order kept back as one given, and the store reads a prompt's row
/// without them as no work order.
pub const FEATURES: GateFeatures = GateFeatures {
    top_lexical: 1.0,
    top_mass: 0.5,
    agreeing_kinds: 2,
    supporting_kinds: 2,
    similar_task: 0.0,
    recent_precision: None,
    is_continuing: false,
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

/// A repository in a temporary folder with its store. Fields drop in order: the writer closes the
/// store's files before the folder is removed, which Windows refuses while they are open.
pub struct Repo {
    /// The store's writer.
    pub writer: Writer,
    /// The repository's root folder.
    pub root: tempfile::TempDir,
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
        let native = RepoPath::new(path)?.to_native(self.root.path());
        if let Some(folder) = native.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&native, text)?;
        self.stored(path, text, defs, flags)
    }

    /// Stores the file at `path` as [`Repo::file`] does, holding `text`, without writing it to
    /// disk: for a name not every file system takes, such as one holding `<`.
    pub fn stored(
        &mut self,
        path: &str,
        text: &str,
        defs: &[Def<'_>],
        flags: FileFlags,
    ) -> TestResult<Vec<NodeId>> {
        let path = RepoPath::new(path)?;
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

    /// Deletes what the store holds for the file at `path`, its node included.
    pub fn remove(&mut self, path: &str) -> TestResult {
        let mut transaction = self.writer.transaction()?;
        transaction.remove_file(&RepoPath::new(path)?)?;
        transaction.commit()?;
        Ok(())
    }

    /// Stores `clusters` as the clusters of earlier tasks, replacing any: each its words, best
    /// first, its files and how many tasks it covers.
    pub fn clusters(&mut self, clusters: &[(&[&str], &[NodeId], u32)]) -> TestResult {
        let records = clusters
            .iter()
            .map(|(words, files, episode_count)| {
                Ok(ArchetypeRecord {
                    seed_terms: words
                        .iter()
                        .map(|word| Identifier::new(word))
                        .collect::<Result<_, _>>()?,
                    files: files.to_vec(),
                    episode_count: *episode_count,
                    updated_at_ms: 1,
                })
            })
            .collect::<TestResult<Vec<ArchetypeRecord>>>()?;
        let mut transaction = self.writer.transaction()?;
        transaction.replace_archetypes(&records)?;
        transaction.commit()?;
        Ok(())
    }

    /// Records `mode` as the repository's mode, as the first index does.
    pub fn mode(&mut self, mode: ScaleMode) -> TestResult {
        let mut transaction = self.writer.transaction()?;
        transaction.set_scale_mode(mode)?;
        transaction.commit()?;
        Ok(())
    }

    /// Records, as the git miner counts them, the commits each of `commits` changed in, by path,
    /// and the commits that changed both files of each of `pairs`. Each count is added once, as
    /// the store adds counts to those it holds.
    pub fn cochange(
        &mut self,
        commits: &[(&str, u32)],
        pairs: &[((&str, &str), u32)],
    ) -> TestResult {
        let mut transaction = self.writer.transaction()?;
        let id = |path: &str| -> TestResult<cortex_store::FileId> {
            Ok(transaction
                .queries()
                .file(&RepoPath::new(path)?)?
                .ok_or("a stored file")?
                .id)
        };
        let mut files = Vec::new();
        for (path, count) in commits {
            files.push(cortex_store::FileCommitsRow {
                file: id(path)?,
                count: *count,
                weighted: f64::from(*count),
            });
        }
        let mut together = Vec::new();
        for ((a, b), count) in pairs {
            let (a, b) = (id(a)?, id(b)?);
            together.push(cortex_store::PairRow {
                file_a: a.min(b),
                file_b: a.max(b),
                count: *count,
                weighted: f64::from(*count),
                last_commit_seq: 1,
            });
        }
        transaction.add_cochange(&files, &together)?;
        transaction.commit()?;
        Ok(())
    }

    /// A reader of the store.
    pub fn reader(&self) -> TestResult<Reader> {
        Ok(Reader::open(&self.root.path().join(".cortex"), 0)?)
    }
}

/// An event of `hook` in `session` at `ts_ms`.
pub fn event(session: &str, ts_ms: i64, hook: HookKind) -> TestResult<Event> {
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, session.as_bytes()),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new(session)?,
        parent_session_id: None,
        cwd: std::path::PathBuf::from("/repo"),
        repo_root: std::path::PathBuf::from("/repo"),
        worktree_root: None,
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: (hook == HookKind::Prompt).then(|| "fix it".to_owned()),
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// A tool's result in `session` at `ts_ms`: `tool` on `path`, the lines `ranges`, done.
pub fn tool(
    (session, ts_ms): (&str, i64),
    tool: ToolKind,
    (path, ranges): (&str, &[(u32, u32)]),
) -> TestResult<Event> {
    Ok(Event {
        tool: Some(tool),
        tool_target: Some(ToolTarget {
            path: Some(RepoPath::new(path)?),
            ranges: ranges
                .iter()
                .map(|(start, end)| LineRange::new(*start, *end))
                .collect::<Result<_, _>>()?,
            ..ToolTarget::default()
        }),
        tool_result: Some(ToolResult {
            exit_code: None,
            is_success: true,
            is_interrupted: false,
            output_bytes: None,
            counts: None,
        }),
        ..event(session, ts_ms, HookKind::PostTool)?
    })
}

/// `event`, failed.
pub fn failed(event: Event) -> Event {
    Event {
        tool_result: Some(ToolResult {
            exit_code: None,
            is_success: false,
            is_interrupted: false,
            output_bytes: None,
            counts: None,
        }),
        ..event
    }
}

/// A subagent's event: its session is `parent/agent`, and its parent is `parent`.
pub fn of_subagent(parent: &str, event: Event) -> TestResult<Event> {
    Ok(Event {
        session_id: Identifier::new(&format!("{parent}/agent"))?,
        parent_session_id: Some(Identifier::new(parent)?),
        ..event
    })
}

/// Records `events` into `repo`'s store, each in a transaction of its own.
pub fn record(repo: &mut Repo, events: &[Event]) -> TestResult {
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    for event in events {
        let mut transaction = repo.writer.transaction()?;
        recorder.record(&mut transaction, event)?;
        transaction.commit()?;
    }
    Ok(())
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
