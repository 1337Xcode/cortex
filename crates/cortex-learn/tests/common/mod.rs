//! Helpers shared by the recorder's tests: events in the shapes Claude Code sends, and a store in
//! a temporary repository. Each test file uses part of them.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

use std::error::Error;
use std::path::PathBuf;

use cortex_core::{
    Agent, ContentHash, EdgeType, Event, EventKind, EventMeta, FileFlags, GateFeatures, HookKind,
    Identifier, Language, LineRange, RepoId, RepoPath, ShellCommand, ShortHash, StartSource,
    SymbolKind, ToolKind, ToolResult, ToolTarget, Ulid,
};
use cortex_learn::{Recorder, TaskState};
use cortex_store::{
    EdgeRecord, EventRecord, FileIndex, FileRecord, InjectedFile, InjectionRecord, NodeId,
    SymbolRecord, Writer, WriterOptions,
};

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// A name.
pub fn name(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
}

/// A name, under a name that does not shadow a local `name`.
fn name_of(text: &str) -> TestResult<Identifier> {
    name(text)
}

/// An event of `hook` from session `session` at `ts_ms`, nothing else known.
pub fn event(hook: HookKind, session: &str, ts_ms: i64) -> TestResult<Event> {
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, session.as_bytes()),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: name(session)?,
        parent_session_id: None,
        cwd: PathBuf::from("/repo"),
        repo_root: PathBuf::from("/repo"),
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: None,
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// A session start of `source`.
pub fn start(session: &str, ts_ms: i64, source: StartSource) -> TestResult<Event> {
    Ok(Event {
        source: Some(source),
        ..event(HookKind::SessionStart, session, ts_ms)?
    })
}

/// A prompt of `text`.
pub fn prompt(session: &str, ts_ms: i64, text: &str) -> TestResult<Event> {
    Ok(Event {
        prompt_text: Some(text.to_owned()),
        ..event(HookKind::Prompt, session, ts_ms)?
    })
}

/// A session end for `reason`.
pub fn end(session: &str, ts_ms: i64, reason: &str) -> TestResult<Event> {
    Ok(Event {
        end_reason: Some(name(reason)?),
        ..event(HookKind::SessionEnd, session, ts_ms)?
    })
}

/// A tool event of `hook` for `tool` aimed at `target`, with `result` once it ran.
pub fn tool(
    hook: HookKind,
    (session, ts_ms): (&str, i64),
    (tool, target, result): (ToolKind, ToolTarget, Option<ToolResult>),
) -> TestResult<Event> {
    Ok(Event {
        tool: Some(tool),
        tool_target: Some(target),
        tool_result: result,
        ..event(hook, session, ts_ms)?
    })
}

/// A target at `path` covering `ranges`.
pub fn target(path: &str, ranges: &[(u32, u32)]) -> TestResult<ToolTarget> {
    Ok(ToolTarget {
        path: Some(RepoPath::new(path)?),
        ranges: ranges
            .iter()
            .map(|(start, end)| LineRange::new(*start, *end))
            .collect::<Result<_, _>>()?,
        cut_after: None,
        pattern: None,
        command: None,
        hits: Vec::new(),
        more_paths: Vec::new(),
        failed_output: None,
    })
}

/// A tool that succeeded, its exit code `exit_code` when one is reported.
pub fn succeeded(exit_code: Option<i32>, output_bytes: Option<u64>) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code,
        is_success: true,
        is_interrupted: false,
        output_bytes,
        counts: None,
    })
}

/// A tool that failed, as Claude Code reports an edit that did not apply: no exit code, no output
/// size.
pub fn failed() -> Option<ToolResult> {
    Some(ToolResult {
        exit_code: None,
        is_success: false,
        is_interrupted: false,
        output_bytes: None,
        counts: None,
    })
}

/// The shell command `line` run by `session` at `ts_ms` in the repository's top folder, which
/// exited `exit_code`, or was interrupted before it finished for none. The root is absolute on
/// every platform, so that the paths a restore names resolve.
pub fn shell(session: &str, ts_ms: i64, line: &str, exit_code: Option<i32>) -> TestResult<Event> {
    let root = std::env::temp_dir().join("repo");
    let target = ToolTarget {
        command: Some(ShellCommand::cut(line)),
        ..ToolTarget::default()
    };
    let result = Some(ToolResult {
        exit_code,
        is_success: exit_code == Some(0),
        is_interrupted: exit_code.is_none(),
        output_bytes: Some(20),
        counts: None,
    });
    Ok(Event {
        cwd: root.clone(),
        repo_root: root,
        ..tool(
            HookKind::PostTool,
            (session, ts_ms),
            (ToolKind::Shell, target, result),
        )?
    })
}

/// A store in a temporary repository. Fields drop in order: the writer closes the store's files
/// before the folder is removed, which Windows refuses while they are open.
pub struct Store {
    /// The writer.
    pub writer: Writer,
    /// The repository root.
    pub root: tempfile::TempDir,
}

impl Store {
    /// An empty store.
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

    /// Records `events` with `recorder`, each in a transaction of its own, and returns the states
    /// of the tasks they ended, in order.
    pub fn record(
        &mut self,
        recorder: &mut Recorder,
        events: &[Event],
    ) -> TestResult<Vec<TaskState>> {
        let mut ended = Vec::new();
        for event in events {
            let mut transaction = self.writer.transaction()?;
            ended.extend(recorder.record(&mut transaction, event)?);
            transaction.commit()?;
        }
        Ok(ended)
    }

    /// Stores the files at `paths` as the indexer would, with no language and contents not read,
    /// and returns their nodes in that order.
    pub fn index(&mut self, paths: &[&str]) -> TestResult<Vec<NodeId>> {
        let unread: Vec<(&str, Option<ContentHash>)> =
            paths.iter().map(|path| (*path, None)).collect();
        self.store_files(&unread, 1)
    }

    /// Stores each file of `files` as the indexer would after reading its text, and returns their
    /// nodes in that order.
    pub fn index_text(&mut self, files: &[(&str, &str)]) -> TestResult<Vec<NodeId>> {
        let read: Vec<(&str, Option<ContentHash>)> = files
            .iter()
            .map(|(path, text)| (*path, Some(ContentHash::of(text.as_bytes()))))
            .collect();
        self.store_files(&read, 1)
    }

    /// Stores each file of `files` as the indexer would after reading its text at
    /// `indexed_at_ms`, and returns their nodes in that order.
    pub fn index_text_at(
        &mut self,
        files: &[(&str, &str)],
        indexed_at_ms: i64,
    ) -> TestResult<Vec<NodeId>> {
        let read: Vec<(&str, Option<ContentHash>)> = files
            .iter()
            .map(|(path, text)| (*path, Some(ContentHash::of(text.as_bytes()))))
            .collect();
        self.store_files(&read, indexed_at_ms)
    }

    /// Stores the file at `path` with one function symbol per entry of `symbols`, each named and
    /// spanning the lines given, and returns the file's node and then each symbol's node in that
    /// order. The folder holding the file is given a node too, as the index gives it one.
    pub fn index_symbols(
        &mut self,
        path: &str,
        symbols: &[(&str, u32, u32)],
    ) -> TestResult<Vec<NodeId>> {
        let path = RepoPath::new(path)?;
        let records: Vec<SymbolRecord> = symbols
            .iter()
            .map(|(name, start_line, end_line)| {
                Ok(SymbolRecord {
                    parent: None,
                    kind: SymbolKind::Function,
                    name: name_of(name)?,
                    qual_name: name_of(name)?,
                    start_byte: u64::from(*start_line),
                    end_byte: u64::from(*end_line),
                    start_line: *start_line,
                    end_line: *end_line,
                    sig_hash: ShortHash::of(name.as_bytes()),
                    body_hash: ShortHash::of(name.as_bytes()),
                })
            })
            .collect::<TestResult<Vec<SymbolRecord>>>()?;
        let mut transaction = self.writer.transaction()?;
        transaction.replace_file(&FileIndex {
            file: FileRecord {
                path: path.clone(),
                language: Some(Language::Python),
                size_bytes: 100,
                line_count: Some(100),
                content_hash: Some(ContentHash::of(path.as_str().as_bytes())),
                mtime_ns: 1,
                flags: FileFlags::PARSED,
                indexed_at_ms: 1,
            },
            symbols: records,
            refs: Vec::new(),
            passes_on: Vec::new(),
        })?;
        let mut nodes = vec![
            transaction
                .queries()
                .file_node_at(&path)?
                .ok_or("no node for a stored file")?,
        ];
        if let Some(folder) = path.parent().filter(|folder| !folder.is_root()) {
            transaction.folder_node(&folder)?;
        }
        let file = transaction
            .queries()
            .file(&path)?
            .ok_or("no row for a stored file")?
            .id;
        nodes.extend(
            transaction
                .queries()
                .symbol_spans_in_file(file)?
                .into_iter()
                .map(|span| span.node),
        );
        transaction.commit()?;
        Ok(nodes)
    }

    /// Records a whole task of `session` that changed each file of `paths`, stores its record as a
    /// success, and returns the task's id, as [`Store::task_ended_as`] does.
    pub fn task_that_changed(
        &mut self,
        recorder: &mut Recorder,
        at: (&str, i64),
        paths: &[&str],
    ) -> TestResult<Ulid> {
        self.task_ended_as(recorder, at, paths, cortex_core::TaskOutcome::Success)
    }

    /// Records a whole task of `session` that changed each file of `paths`, stores its record
    /// with `outcome`, and returns the task's id. The session is started again with each task, as
    /// a session that resumes is, and the task ends with its answer.
    pub fn task_ended_as(
        &mut self,
        recorder: &mut Recorder,
        (session, at_ms): (&str, i64),
        paths: &[&str],
        outcome: cortex_core::TaskOutcome,
    ) -> TestResult<Ulid> {
        let mut events = vec![
            start(session, at_ms, StartSource::Startup)?,
            prompt(session, at_ms + 1, "change it")?,
        ];
        for (at, path) in paths.iter().enumerate() {
            let at_ms = at_ms + 2 + i64::try_from(at)?;
            events.push(tool(
                HookKind::PostTool,
                (session, at_ms),
                (
                    ToolKind::Edit,
                    target(path, &[(1, 1)])?,
                    succeeded(None, None),
                ),
            )?);
        }
        events.push(event(HookKind::Stop, session, at_ms + 400)?);
        let mut ended = self.record(recorder, &events)?.into_iter();
        let state = ended.next().ok_or("no task ended")?;
        let record =
            cortex_learn::episode_of(self.writer.queries(), &state, (outcome, at_ms + 500))?;
        let mut transaction = self.writer.transaction()?;
        transaction.replace_episode(&record)?;
        transaction.commit()?;
        Ok(state.task_id())
    }

    /// Writes a structural edge from `from` to `to`, as the index writes one for a file whose
    /// imports name another file, or for a symbol that names another.
    pub fn names(&mut self, from: NodeId, to: NodeId) -> TestResult {
        let mut transaction = self.writer.transaction()?;
        transaction.set_edges(
            EdgeType::Structural,
            &[(
                from,
                EdgeRecord {
                    dst: to,
                    weight: 0.8,
                    count: 1,
                    last_seq: 0,
                    is_directed: true,
                },
            )],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// The node of the folder at `path`, added as the index adds it.
    pub fn folder_node(&mut self, path: &str) -> TestResult<NodeId> {
        let mut transaction = self.writer.transaction()?;
        let node = transaction.folder_node(&RepoPath::new(path)?)?;
        transaction.commit()?;
        Ok(node)
    }

    /// Stores each file of `files` with its contents' hash, if read, as indexed at `indexed_at_ms`,
    /// and returns their nodes in that order.
    fn store_files(
        &mut self,
        files: &[(&str, Option<ContentHash>)],
        indexed_at_ms: i64,
    ) -> TestResult<Vec<NodeId>> {
        let mut transaction = self.writer.transaction()?;
        let mut nodes = Vec::new();
        for (path, content_hash) in files {
            let path = RepoPath::new(path)?;
            transaction.replace_file(&FileIndex {
                file: FileRecord {
                    path: path.clone(),
                    language: None,
                    size_bytes: 10,
                    line_count: None,
                    content_hash: *content_hash,
                    mtime_ns: 1,
                    flags: FileFlags::empty(),
                    indexed_at_ms,
                },
                symbols: Vec::new(),
                refs: Vec::new(),
                passes_on: Vec::new(),
            })?;
            nodes.push(
                transaction
                    .queries()
                    .file_node_at(&path)?
                    .ok_or("no node for a stored file")?,
            );
        }
        transaction.commit()?;
        Ok(nodes)
    }

    /// Every event stored, in order, as (kind, task, session, path, range, meta) for comparing.
    pub fn events(&self) -> TestResult<Vec<EventRecord>> {
        Ok(self
            .writer
            .queries()
            .events_after(0, 1_000)?
            .into_iter()
            .map(|row| row.record)
            .collect())
    }

    /// The folder of the store's files.
    pub fn folder(&self) -> PathBuf {
        self.root.path().join(".cortex")
    }
}

/// The row an event of `kind` from `session` at `ts_ms` is stored as, in `task`, with nothing
/// else.
pub fn row(
    kind: EventKind,
    session: &str,
    ts_ms: i64,
    task: Option<Ulid>,
) -> TestResult<EventRecord> {
    Ok(EventRecord {
        task_id: task,
        session_id: Some(name(session)?),
        ts_ms,
        kind,
        node_id: None,
        path: None,
        range: None,
        meta: EventMeta::default(),
    })
}

/// The id the recorder gives the task the prompt `event` starts: made from the prompt's time and
/// its event id's text.
pub fn task_of(event: &Event) -> TestResult<Ulid> {
    Ok(Ulid::from_seed(
        u64::try_from(event.ts_ms)?,
        event.event_id.to_string().as_bytes(),
    ))
}

/// A read by `session` at `ts_ms` of `path`, the lines `range`, or of the whole file for none.
pub fn read(session: &str, ts_ms: i64, path: &str, range: &[(u32, u32)]) -> TestResult<Event> {
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (
            ToolKind::Read,
            target(path, range)?,
            succeeded(None, Some(80)),
        ),
    )
}

/// An edit by `session` at `ts_ms` of `path`, of the lines `range`, that went through.
pub fn edit(session: &str, ts_ms: i64, path: &str, range: &[(u32, u32)]) -> TestResult<Event> {
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (ToolKind::Edit, target(path, range)?, succeeded(None, None)),
    )
}

/// The state of one task: a session started at `at_ms`, a prompt, then `steps`, then the end of
/// the answer.
pub fn answered_task(
    (store, recorder): (&mut Store, &mut Recorder),
    (session, at_ms): (&str, i64),
    steps: &[Event],
) -> TestResult<TaskState> {
    asked_task((store, recorder), (session, at_ms), "fix it", steps)
}

/// The state of one task: a session started at `at_ms`, a prompt of `text`, then `steps`, then the
/// end of the answer.
pub fn asked_task(
    (store, recorder): (&mut Store, &mut Recorder),
    (session, at_ms): (&str, i64),
    text: &str,
    steps: &[Event],
) -> TestResult<TaskState> {
    let mut events = vec![
        start(session, at_ms, StartSource::Startup)?,
        prompt(session, at_ms + 1, text)?,
    ];
    events.extend(steps.iter().cloned());
    events.push(event(HookKind::Stop, session, at_ms + 1_000)?);
    let mut ended = store.record(recorder, &events)?.into_iter();
    let state = ended.next().ok_or("no task ended")?;
    assert!(ended.next().is_none(), "more than one task ended");
    Ok(state)
}

/// How an injection's text fared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Given {
    /// It reached the agent.
    Shown,
    /// The gate kept it back, so there was no text.
    KeptBack,
    /// The task was held out: the text was recorded, and not given.
    HeldOut,
}

/// One file of an injection, with the first and last lines of each of its spans.
pub type GivenSpans<'a> = (NodeId, &'a [(u32, u32)]);

/// An injection for the task `task`, which a prompt of `session` began at `at_ms`, of `files`, the
/// best first, its text faring as `given` says: a work order, with the gate's features, as every
/// prompt's retrieval records it.
pub fn injection(
    (session, task, at_ms): (&str, Ulid, i64),
    files: &[GivenSpans<'_>],
    given: Given,
) -> TestResult<InjectionRecord> {
    Ok(InjectionRecord {
        task_id: Some(task),
        session_id: Identifier::new(session)?,
        hook: HookKind::Prompt,
        at_ms,
        files: files
            .iter()
            .map(|(node, spans)| InjectedFile {
                node: *node,
                spans: spans.to_vec(),
                reason: None,
                shown: Vec::new(),
                content: None,
            })
            .collect(),
        tokens: 50,
        confidence: None,
        is_gated_out: given == Given::KeptBack,
        is_holdout: given == Given::HeldOut,
        text_hash: (given != Given::KeptBack).then(|| ContentHash::of(b"the work order")),
        features: Some(GateFeatures {
            top_lexical: 1.0,
            top_mass: 0.5,
            agreeing_kinds: 2,
            supporting_kinds: 2,
            similar_task: 0.0,
            recent_precision: None,
            is_continuing: false,
        }),
    })
}

/// The state of one task whose prompt got an injection: a session started at `at_ms`, a prompt,
/// an injection of `files` faring as `given` says, then `steps` and the end of the answer, recorded
/// as the daemon records them.
pub fn given_task(
    (store, recorder): (&mut Store, &mut Recorder),
    (session, at_ms): (&str, i64),
    (files, given): (&[GivenSpans<'_>], Given),
    steps: &[Event],
) -> TestResult<TaskState> {
    let asked = prompt(session, at_ms + 1, "fix it")?;
    let started = start(session, at_ms, StartSource::Startup)?;
    store.record(recorder, &[started, asked.clone()])?;
    let task = task_of(&asked)?;
    let mut transaction = store.writer.transaction()?;
    transaction.record_injection(&injection((session, task, at_ms + 1), files, given)?)?;
    transaction.commit()?;
    let mut rest = steps.to_vec();
    rest.push(event(HookKind::Stop, session, at_ms + 1_000)?);
    let mut ended = store.record(recorder, &rest)?.into_iter();
    let state = ended.next().ok_or("no task ended")?;
    assert!(ended.next().is_none(), "more than one task ended");
    Ok(state)
}
