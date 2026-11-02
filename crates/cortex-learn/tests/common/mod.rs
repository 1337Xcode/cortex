//! Helpers shared by the recorder's tests: events in the shapes Claude Code sends, and a store in
//! a temporary repository. Each test file uses part of them.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

use std::error::Error;
use std::path::PathBuf;

use cortex_core::{
    Agent, Event, EventKind, EventMeta, FileFlags, HookKind, Identifier, LineRange, RepoId,
    RepoPath, StartSource, ToolKind, ToolResult, ToolTarget, Ulid,
};
use cortex_learn::{Recorder, TaskState};
use cortex_store::{EventRecord, FileIndex, FileRecord, NodeId, Writer, WriterOptions};

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// A name.
pub fn name(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
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
        pattern: None,
        command: None,
        hits: Vec::new(),
    })
}

/// A tool that succeeded, its exit code `exit_code` when one is reported.
pub fn succeeded(exit_code: Option<i32>, output_bytes: Option<u64>) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code,
        is_success: true,
        is_interrupted: false,
        output_bytes,
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
    })
}

/// A store in a temporary repository.
pub struct Store {
    /// The repository root.
    pub root: tempfile::TempDir,
    /// The writer.
    pub writer: Writer,
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

    /// Stores the files at `paths` as the indexer would, with no language, and returns their
    /// nodes in that order.
    pub fn index(&mut self, paths: &[&str]) -> TestResult<Vec<NodeId>> {
        let mut transaction = self.writer.transaction()?;
        let mut nodes = Vec::new();
        for path in paths {
            let path = RepoPath::new(path)?;
            transaction.replace_file(&FileIndex {
                file: FileRecord {
                    path: path.clone(),
                    language: None,
                    size_bytes: 10,
                    line_count: None,
                    content_hash: None,
                    mtime_ns: 1,
                    flags: FileFlags::empty(),
                    indexed_at_ms: 1,
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
