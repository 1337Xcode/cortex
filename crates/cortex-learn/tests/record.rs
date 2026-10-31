//! Sequences of events, in the orders Claude Code sends them, recorded into a real store; the rows
//! expected are written out by hand.

use std::error::Error;
use std::path::{Path, PathBuf};

use cortex_core::{
    Agent, CompactTrigger, ContentHash, Event, EventKind, EventMeta, FileFlags, HookKind,
    Identifier, Language, LineRange, RepoId, RepoPath, SessionStatus, ShellCommand, StartSource,
    TaskOutcome, ToolKind, ToolResult, ToolTarget, Ulid,
};
use cortex_learn::Recorder;
use cortex_store::{EventRecord, FileIndex, FileRecord, Writer, WriterOptions};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// A name.
fn name(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
}

/// An event of `hook` from session `session` at `ts_ms`, nothing else known.
fn event(hook: HookKind, session: &str, ts_ms: i64) -> TestResult<Event> {
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
    })
}

/// A session start of `source`.
fn start(session: &str, ts_ms: i64, source: StartSource) -> TestResult<Event> {
    Ok(Event {
        source: Some(source),
        ..event(HookKind::SessionStart, session, ts_ms)?
    })
}

/// A prompt of `text`.
fn prompt(session: &str, ts_ms: i64, text: &str) -> TestResult<Event> {
    Ok(Event {
        prompt_text: Some(text.to_owned()),
        ..event(HookKind::Prompt, session, ts_ms)?
    })
}

/// A session end for `reason`.
fn end(session: &str, ts_ms: i64, reason: &str) -> TestResult<Event> {
    Ok(Event {
        end_reason: Some(name(reason)?),
        ..event(HookKind::SessionEnd, session, ts_ms)?
    })
}

/// A tool event of `hook` for `tool` aimed at `target`, with `result` once it ran.
fn tool(
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
fn target(path: &str, ranges: &[(u32, u32)]) -> TestResult<ToolTarget> {
    Ok(ToolTarget {
        path: Some(RepoPath::new(path)?),
        ranges: ranges
            .iter()
            .map(|(start, end)| LineRange::new(*start, *end))
            .collect::<Result<_, _>>()?,
        pattern: None,
        command: None,
    })
}

/// A tool that succeeded, its exit code `exit_code` when one is reported.
fn succeeded(exit_code: Option<i32>, output_bytes: Option<u64>) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code,
        is_success: true,
        is_interrupted: false,
        output_bytes,
    })
}

/// A store in a temporary repository.
struct Store {
    /// The repository root.
    root: tempfile::TempDir,
    /// The writer.
    writer: Writer,
}

impl Store {
    /// An empty store.
    fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let writer = Writer::open(&WriterOptions {
            folder: &root.path().join(".cortex"),
            repo_id: &RepoId::of(root.path()),
            mmap_bytes: 0,
            now_ms: 1,
        })?;
        Ok(Self { root, writer })
    }

    /// Records `events` with `recorder`, each in a transaction of its own.
    fn record(&mut self, recorder: &mut Recorder, events: &[Event]) -> TestResult {
        for event in events {
            let mut transaction = self.writer.transaction()?;
            recorder.record(&mut transaction, event)?;
            transaction.commit()?;
        }
        Ok(())
    }

    /// Every event stored, in order, as (kind, task, session, path, range, meta) for comparing.
    fn events(&self) -> TestResult<Vec<EventRecord>> {
        Ok(self
            .writer
            .queries()
            .events_after(0, 1_000)?
            .into_iter()
            .map(|row| row.record)
            .collect())
    }

    /// The folder of the store's files.
    fn folder(&self) -> PathBuf {
        self.root.path().join(".cortex")
    }
}

/// The row an event of `kind` from `session` at `ts_ms` is stored as, in `task`, with nothing
/// else.
fn row(kind: EventKind, session: &str, ts_ms: i64, task: Option<Ulid>) -> TestResult<EventRecord> {
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
fn task_of(event: &Event) -> TestResult<Ulid> {
    Ok(Ulid::from_seed(
        u64::try_from(event.ts_ms)?,
        event.event_id.to_string().as_bytes(),
    ))
}

/// A whole task, as one headless run sends it: the session starts, a prompt starts task 1, the
/// read before it ran is not kept, the read, the edit and the passing command after they ran are,
/// Stop ends the task and SessionEnd the session.
#[test]
fn a_task_from_prompt_to_stop() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let started = Event {
        model: Some(name("sonnet")?),
        transcript_path: Some(PathBuf::from("/home/dev/s1.jsonl")),
        ..start("s1", 1_000, StartSource::Startup)?
    };
    let asked = prompt("s1", 2_000, "fix divide")?;
    let task = task_of(&asked)?;
    let read = target("calc/ops.py", &[(2, 5)])?;
    let shell = ToolTarget {
        command: Some(ShellCommand::cut("pytest -q")),
        ..ToolTarget::default()
    };
    let events = [
        started,
        asked,
        tool(
            HookKind::PreTool,
            ("s1", 2_100),
            (ToolKind::Read, read.clone(), None),
        )?,
        tool(
            HookKind::PostTool,
            ("s1", 2_200),
            (ToolKind::Read, read, None),
        )?,
        tool(
            HookKind::PostTool,
            ("s1", 2_300),
            (
                ToolKind::Edit,
                target("calc/ops.py", &[(10, 13)])?,
                succeeded(None, None),
            ),
        )?,
        tool(
            HookKind::PostTool,
            ("s1", 2_400),
            (ToolKind::Shell, shell, succeeded(Some(0), Some(42))),
        )?,
        event(HookKind::Stop, "s1", 2_500)?,
        end("s1", 2_600, "other")?,
    ];
    store.record(&mut recorder, &events)?;
    let session = store.writer.queries().session(&name("s1")?)?.ok_or("s1")?;
    assert_eq!(session.record.model, Some(name("sonnet")?));
    assert_eq!(session.record.started_at_ms, 1_000);
    assert_eq!(
        session.record.transcript_path,
        Some(PathBuf::from("/home/dev/s1.jsonl"))
    );
    assert_eq!(
        (session.ended_at_ms, session.status),
        (Some(2_600), SessionStatus::Ended)
    );
    let stored = store.writer.queries().task(task)?.ok_or("task")?;
    assert_eq!(stored.seq, 1);
    assert_eq!(
        (
            stored.record.started_at_ms,
            stored.ended_at_ms,
            stored.outcome
        ),
        (2_000, Some(2_500), Some(TaskOutcome::Unknown))
    );
    assert_eq!(
        stored.record.prompt_hash,
        Some(ContentHash::of(b"fix divide"))
    );
    assert_eq!(stored.record.prompt_excerpt, None);
    let path = Some(RepoPath::new("calc/ops.py")?);
    let expected = [
        EventRecord {
            meta: EventMeta {
                start_source: Some(StartSource::Startup),
                ..EventMeta::default()
            },
            ..row(EventKind::SessionStart, "s1", 1_000, None)?
        },
        row(EventKind::Prompt, "s1", 2_000, Some(task))?,
        EventRecord {
            path: path.clone(),
            range: Some(LineRange::new(2, 5)?),
            meta: EventMeta {
                tool: Some(ToolKind::Read),
                ..EventMeta::default()
            },
            ..row(EventKind::Read, "s1", 2_200, Some(task))?
        },
        EventRecord {
            path,
            range: Some(LineRange::new(10, 13)?),
            meta: EventMeta {
                tool: Some(ToolKind::Edit),
                is_success: Some(true),
                is_interrupted: Some(false),
                ..EventMeta::default()
            },
            ..row(EventKind::Edit, "s1", 2_300, Some(task))?
        },
        EventRecord {
            meta: EventMeta {
                tool: Some(ToolKind::Shell),
                exit_code: Some(0),
                is_success: Some(true),
                is_interrupted: Some(false),
                output_bytes: Some(42),
                ..EventMeta::default()
            },
            ..row(EventKind::Shell, "s1", 2_400, Some(task))?
        },
        row(EventKind::Stop, "s1", 2_500, Some(task))?,
        EventRecord {
            meta: EventMeta {
                end_reason: Some(name("other")?),
                ..EventMeta::default()
            },
            ..row(EventKind::SessionEnd, "s1", 2_600, None)?
        },
    ];
    assert_eq!(store.events()?, expected);
    Ok(())
}

/// `/compact` in the order captured: the session resumes, PreCompact, Claude Code's own
/// compaction agent stops (a subagent session never started, registered on its stop and ended),
/// the session starts again from the compaction, then ends. The open task carries through: the
/// compaction rows belong to it, and the session stays one session.
#[test]
fn compaction_keeps_the_open_task() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let asked = prompt("s1", 100, "refactor parse")?;
    let task = Some(task_of(&asked)?);
    let compact = Event {
        compact_trigger: Some(CompactTrigger::Manual),
        ..event(HookKind::Compact, "s1", 300)?
    };
    let internal_stop = Event {
        parent_session_id: Some(name("s1")?),
        ..event(HookKind::SubagentStop, "s1/agent1", 400)?
    };
    let events = [
        start("s1", 50, StartSource::Startup)?,
        asked,
        start("s1", 200, StartSource::Resume)?,
        compact,
        internal_stop,
        start("s1", 500, StartSource::Compact)?,
        end("s1", 600, "other")?,
    ];
    store.record(&mut recorder, &events)?;
    let kinds: Vec<(EventKind, Option<Ulid>, String)> = store
        .events()?
        .into_iter()
        .map(|record| {
            let session = record
                .session_id
                .map(|id| id.as_str().to_owned())
                .unwrap_or_default();
            (record.kind, record.task_id, session)
        })
        .collect();
    let s1 = "s1".to_owned();
    assert_eq!(
        kinds,
        [
            (EventKind::SessionStart, None, s1.clone()),
            (EventKind::Prompt, task, s1.clone()),
            (EventKind::SessionStart, task, s1.clone()),
            (EventKind::Compact, task, s1.clone()),
            (EventKind::SubagentStop, task, "s1/agent1".to_owned()),
            (EventKind::SessionStart, task, s1.clone()),
            (EventKind::SessionEnd, task, s1),
        ]
    );
    let internal = store
        .writer
        .queries()
        .session(&name("s1/agent1")?)?
        .ok_or("the compaction agent")?;
    assert_eq!(internal.record.parent_session_id, Some(name("s1")?));
    assert_eq!(internal.status, SessionStatus::Ended);
    let events = store.events()?;
    let compacted = events
        .iter()
        .find(|record| record.kind == EventKind::Compact)
        .ok_or("compact row")?;
    assert_eq!(compacted.meta.compact_trigger, Some(CompactTrigger::Manual));
    Ok(())
}

/// `/clear` in the order captured: the old session resumes and ends with reason `clear`, a new
/// session starts from the clear and ends. Two sessions, each ended.
#[test]
fn clearing_ends_one_session_and_starts_another() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let events = [
        start("s1", 10, StartSource::Resume)?,
        end("s1", 20, "clear")?,
        start("s2", 30, StartSource::Clear)?,
        end("s2", 40, "other")?,
    ];
    store.record(&mut recorder, &events)?;
    let queries = store.writer.queries();
    let first = queries.session(&name("s1")?)?.ok_or("s1")?;
    let second = queries.session(&name("s2")?)?.ok_or("s2")?;
    assert_eq!(
        (first.record.started_at_ms, first.ended_at_ms, first.status),
        (10, Some(20), SessionStatus::Ended)
    );
    assert_eq!(
        (
            second.record.started_at_ms,
            second.ended_at_ms,
            second.status
        ),
        (30, Some(40), SessionStatus::Ended)
    );
    let ends: Vec<Option<Identifier>> = store
        .events()?
        .into_iter()
        .filter(|record| record.kind == EventKind::SessionEnd)
        .map(|record| record.meta.end_reason)
        .collect();
    assert_eq!(ends, [Some(name("clear")?), Some(name("other")?)]);
    Ok(())
}

/// A subagent is a session of its own with its parent's id; its start, its search and its stop
/// belong to the parent's open task; its stop ends its session, not the parent's task.
#[test]
fn a_subagent_works_inside_its_parent_s_task() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let asked = prompt("s1", 100, "find the parser")?;
    let task = Some(task_of(&asked)?);
    let child = |hook, ts_ms| -> TestResult<Event> {
        Ok(Event {
            parent_session_id: Some(name("s1")?),
            subagent_type: Some(name("Explore")?),
            ..event(hook, "s1/a1", ts_ms)?
        })
    };
    let search = Event {
        parent_session_id: Some(name("s1")?),
        tool: Some(ToolKind::Grep),
        ..event(HookKind::PostTool, "s1/a1", 300)?
    };
    let events = [
        start("s1", 50, StartSource::Startup)?,
        asked,
        child(HookKind::SubagentStart, 200)?,
        search,
        child(HookKind::SubagentStop, 400)?,
        event(HookKind::Stop, "s1", 500)?,
    ];
    store.record(&mut recorder, &events)?;
    let rows: Vec<(EventKind, Option<Ulid>)> = store
        .events()?
        .into_iter()
        .map(|record| (record.kind, record.task_id))
        .collect();
    assert_eq!(
        rows,
        [
            (EventKind::SessionStart, None),
            (EventKind::Prompt, task),
            (EventKind::SubagentStart, task),
            (EventKind::Search, task),
            (EventKind::SubagentStop, task),
            (EventKind::Stop, task),
        ]
    );
    let queries = store.writer.queries();
    let subagent = queries.session(&name("s1/a1")?)?.ok_or("subagent")?;
    assert_eq!(subagent.record.parent_session_id, Some(name("s1")?));
    assert_eq!(
        (subagent.record.started_at_ms, subagent.ended_at_ms),
        (200, Some(400))
    );
    let parent_task = queries.task(task.ok_or("task")?)?.ok_or("task row")?;
    assert_eq!(parent_task.ended_at_ms, Some(500));
    Ok(())
}

/// A prompt that comes while the session's task is still open (its Stop was lost) ends that task
/// as unknown at the prompt's time, and starts the next one; tasks are numbered 1 and 2.
#[test]
fn a_lost_stop_is_closed_by_the_next_prompt() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let first = prompt("s1", 100, "one")?;
    let second = prompt("s1", 200, "two")?;
    let (first_task, second_task) = (task_of(&first)?, task_of(&second)?);
    store.record(&mut recorder, &[first, second])?;
    let queries = store.writer.queries();
    let first_row = queries.task(first_task)?.ok_or("first")?;
    let second_row = queries.task(second_task)?.ok_or("second")?;
    assert_eq!(
        (first_row.seq, first_row.ended_at_ms, first_row.outcome),
        (1, Some(200), Some(TaskOutcome::Unknown))
    );
    assert_eq!(
        (second_row.seq, second_row.ended_at_ms, second_row.outcome),
        (2, None, None)
    );
    Ok(())
}

/// An event from a session the store has never seen registers it, live, started at the event's
/// time, with the event's working folder; the event belongs to no task.
#[test]
fn an_unknown_session_is_registered_by_its_first_event() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let read = tool(
        HookKind::PostTool,
        ("s9", 700),
        (ToolKind::Read, target("a.py", &[])?, None),
    )?;
    store.record(&mut recorder, &[read])?;
    let session = store.writer.queries().session(&name("s9")?)?.ok_or("s9")?;
    assert_eq!(
        (
            session.record.started_at_ms,
            session.status,
            session.record.cwd
        ),
        (700, SessionStatus::Live, Some(PathBuf::from("/repo")))
    );
    let rows: Vec<(EventKind, Option<Ulid>)> = store
        .events()?
        .into_iter()
        .map(|record| (record.kind, record.task_id))
        .collect();
    assert_eq!(rows, [(EventKind::Read, None)]);
    Ok(())
}

/// An edit is one row per changed stretch of lines up to 16 stretches, and one row with no lines
/// past that.
#[test]
fn edits_are_rows_per_stretch_up_to_sixteen() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let two: Vec<(u32, u32)> = vec![(1, 2), (8, 9)];
    let seventeen: Vec<(u32, u32)> = (1..=17).map(|line| (line * 10, line * 10 + 1)).collect();
    let edit = |ts_ms, ranges: &[(u32, u32)]| {
        tool(
            HookKind::PostTool,
            ("s1", ts_ms),
            (ToolKind::Write, target("a.py", ranges)?, None),
        )
    };
    store.record(&mut recorder, &[edit(10, &two)?, edit(20, &seventeen)?])?;
    let ranges: Vec<(i64, Option<LineRange>)> = store
        .events()?
        .into_iter()
        .map(|record| (record.ts_ms, record.range))
        .collect();
    assert_eq!(
        ranges,
        [
            (10, Some(LineRange::new(1, 2)?)),
            (10, Some(LineRange::new(8, 9)?)),
            (20, None),
        ]
    );
    Ok(())
}

/// With excerpts asked for, a task keeps its prompt's first 200 characters; without, none. The
/// database never holds a prompt beyond that, a command or a search pattern: their marker words
/// appear nowhere in the database's files.
#[test]
fn prompts_commands_and_patterns_stay_out_of_the_database() -> TestResult {
    let long_prompt = format!("{}PROMPTTAIL", "p".repeat(200));
    let with_excerpt = prompt("s1", 100, &long_prompt)?;
    let excerpt_task = task_of(&with_excerpt)?;
    let mut store = Store::new()?;
    store.record(&mut Recorder::new(true), &[with_excerpt])?;
    let kept = store.writer.queries().task(excerpt_task)?.ok_or("task")?;
    let excerpt = kept.record.prompt_excerpt.ok_or("an excerpt")?;
    assert_eq!(excerpt.as_str(), "p".repeat(200));
    let secret_prompt = prompt("s1", 200, "PROMPTMARKER7F3A fix the parser")?;
    let shell = ToolTarget {
        command: Some(ShellCommand::cut("echo COMMANDMARKER91C2")),
        pattern: None,
        ..ToolTarget::default()
    };
    let search = ToolTarget {
        pattern: Some("PATTERNMARKER55E1".to_owned()),
        ..target("src", &[])?
    };
    let events = [
        secret_prompt,
        tool(
            HookKind::PostTool,
            ("s1", 300),
            (ToolKind::Shell, shell, succeeded(Some(0), Some(5))),
        )?,
        tool(
            HookKind::PostTool,
            ("s1", 400),
            (ToolKind::Grep, search, succeeded(None, Some(9))),
        )?,
    ];
    let mut recorder = Recorder::new(false);
    store.record(&mut recorder, &events)?;
    let folder = store.folder();
    // Close the database before scanning its files, and keep the temporary folder until the scan
    // is done.
    let Store { root, writer } = store;
    drop(writer);
    for marker in [
        "PROMPTTAIL",
        "PROMPTMARKER7F3A",
        "COMMANDMARKER91C2",
        "PATTERNMARKER55E1",
    ] {
        assert!(
            !files_hold(&folder, marker)?,
            "{marker} reached the database"
        );
    }
    drop(root);
    Ok(())
}

/// Whether any file in `folder` holds the bytes of `text`.
fn files_hold(folder: &Path, text: &str) -> TestResult<bool> {
    for entry in std::fs::read_dir(folder)? {
        let path = entry?.path();
        if path.is_file() {
            let bytes = std::fs::read(&path)?;
            if bytes
                .windows(text.len())
                .any(|window| window == text.as_bytes())
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// A subagent's event that does not name the session that started it still belongs to that
/// session's open task: the store keeps the subagent's parent from its start, and a recorder that
/// remembers nothing (a restarted daemon) reads it there.
#[test]
fn a_subagent_s_parent_is_read_from_the_store() -> TestResult {
    let mut store = Store::new()?;
    let asked = prompt("s1", 100, "find the parser")?;
    let task = Some(task_of(&asked)?);
    let started = Event {
        parent_session_id: Some(name("s1")?),
        ..event(HookKind::SubagentStart, "s1/a1", 200)?
    };
    store.record(
        &mut Recorder::new(false),
        &[start("s1", 50, StartSource::Startup)?, asked, started],
    )?;
    let read = tool(
        HookKind::PostTool,
        ("s1/a1", 300),
        (ToolKind::Read, target("a.py", &[])?, None),
    )?;
    store.record(&mut Recorder::new(false), &[read])?;
    let last = store.events()?.pop().ok_or("the read")?;
    assert_eq!((last.kind, last.task_id), (EventKind::Read, task));
    Ok(())
}

/// A row about an indexed file names the file's graph node, as the store gives it; a row about a
/// folder names none.
#[test]
fn rows_name_an_indexed_file_s_node() -> TestResult {
    let mut store = Store::new()?;
    let path = RepoPath::new("calc/ops.py")?;
    let mut transaction = store.writer.transaction()?;
    transaction.replace_file(&FileIndex {
        file: FileRecord {
            path: path.clone(),
            language: Some(Language::Python),
            size_bytes: 10,
            line_count: Some(1),
            content_hash: Some(ContentHash::of(
                b"x = 1
",
            )),
            mtime_ns: 1,
            flags: FileFlags::empty(),
            indexed_at_ms: 1,
        },
        symbols: Vec::new(),
        refs: Vec::new(),
        passes_on: Vec::new(),
    })?;
    transaction.commit()?;
    let node = store.writer.queries().file_node_at(&path)?;
    assert!(node.is_some());
    let events = [
        tool(
            HookKind::PostTool,
            ("s1", 10),
            (ToolKind::Read, target("calc/ops.py", &[])?, None),
        )?,
        tool(
            HookKind::PostTool,
            ("s1", 20),
            (ToolKind::Grep, target("calc", &[])?, None),
        )?,
    ];
    store.record(&mut Recorder::new(false), &events)?;
    let nodes: Vec<_> = store
        .events()?
        .into_iter()
        .map(|record| record.node_id)
        .collect();
    assert_eq!(nodes, [node, None]);
    Ok(())
}

/// Past the 1,000 sessions a recorder remembers, it forgets them and reads them again: a task
/// opened before 1,000 other sessions started is still found and ended by its session's Stop.
#[test]
fn sessions_past_the_memory_bound_are_read_again() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let asked = prompt("s0", 10, "first")?;
    let task = task_of(&asked)?;
    let mut events = vec![start("s0", 5, StartSource::Startup)?, asked];
    for number in 1..=1_001 {
        events.push(start(
            &format!("s{number}"),
            100 + number,
            StartSource::Startup,
        )?);
    }
    events.push(event(HookKind::Stop, "s0", 5_000)?);
    store.record(&mut recorder, &events)?;
    let ended = store.writer.queries().task(task)?.ok_or("the task")?;
    assert_eq!(
        (ended.ended_at_ms, ended.outcome),
        (Some(5_000), Some(TaskOutcome::Unknown))
    );
    Ok(())
}
