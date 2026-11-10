//! The line naming the file earlier tasks read next, through the daemon's endpoint, on a
//! repository whose store is given ways from one file to another while its daemon is stopped: a
//! read of the first file in a task gets the line, once, recorded as naming the second file for the
//! reason `read_next`, even when several reads are answered at once; a line too late is given at
//! the next read; a held-out task's read gets nothing; the read's own hint and the line together
//! stay within the hint's cap; and a store that cannot say what comes next leaves the read's own
//! hint as it is.

mod common;

use std::path::Path;
use std::sync::Barrier;
use std::time::{Duration, Instant};

use common::{Daemon, StopsDaemon, TestResult, graph_of, wait_for};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    Agent, ContentHash, Event, EventKind, HookKind, Identifier, LineRange, ReasonKind, RepoId,
    RepoPath, RunFiles, ToolKind, ToolTarget, Ulid, UserFolder, canonical_root, task_id_of_prompt,
};
use cortex_hook::ask;
use cortex_store::{InjectedFile, InjectionRow, NodeId, Reader};

/// The file read.
const SERVICE: &str = "src/orders/service.py";

/// The file earlier tasks read after it.
const MODEL: &str = "src/orders/model.py";

/// The text of each file: two lines.
const TEXT: &str = "def f():\n    return 1\n";

/// The line a read of [`SERVICE`] gets naming [`MODEL`], its way taken by 3 tasks.
const LINE: &str = "[cortex] After reading src/orders/service.py, earlier tasks here went on to \
                    read src/orders/model.py (3 times).\n";

/// The read hint of a read of lines 1 and 2 of a file the session read whole: 83 bytes.
const READ_ALREADY: &str =
    "[cortex] You read lines 1-2 of this file earlier in this session; unchanged since.\n";

/// How many reads are sent at once.
const READS_AT_ONCE: usize = 8;

/// A way the store holds from one file to the next: the file read, the file read next, and how
/// many tasks took it, written as the store holds it (a count below 0 is one no store should hold).
type Way<'a> = (&'a str, &'a str, i64);

/// A repository holding the files of `ways`, pinned to medium mode with `holdout_share` of tasks
/// held out, its store holding each way with weight 2.5 as of task 0, written while its daemon is
/// stopped; its home folder, its daemon, and the node of each way's second file.
fn repository(
    holdout_share: &str,
    ways: &[Way<'_>],
) -> TestResult<(tempfile::TempDir, tempfile::TempDir, Daemon, Vec<NodeId>)> {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    for path in ways.iter().flat_map(|(from, to, _)| [from, to]) {
        let native = RepoPath::new(path)?.to_native(root);
        std::fs::create_dir_all(native.parent().ok_or("a folder")?)?;
        std::fs::write(native, TEXT)?;
    }
    std::fs::create_dir_all(root.join(".cortex"))?;
    std::fs::write(
        root.join(".cortex/config.toml"),
        format!("mode = \"medium\"\n[holdout]\nshare = {holdout_share}\n"),
    )?;
    let indexed = Daemon::start(root, home.path())?;
    indexed.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some()))?;
    StopsDaemon::of(root, home.path()).stop()?;
    drop(indexed);
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let node = |path: &str| -> TestResult<NodeId> {
        Ok(reader
            .queries()
            .file_node_at(&RepoPath::new(path)?)?
            .ok_or("an indexed file")?)
    };
    let mut rows = Vec::new();
    for (from, to, count) in ways {
        rows.push((node(from)?, node(to)?, *count));
    }
    drop(reader);
    let store = rusqlite::Connection::open(root.join(".cortex/cortex.db"))?;
    for (src, dst, count) in &rows {
        store.execute(
            "INSERT INTO transitions (src, dst, weight, count, last_seq) VALUES (?1, ?2, 2.5, ?3, 0)",
            (src.get(), dst.get(), count),
        )?;
    }
    drop(store);
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some()))?;
    Ok((
        repo,
        home,
        daemon,
        rows.iter().map(|(_, dst, _)| *dst).collect(),
    ))
}

/// The run files of the daemon of the repository at `root`.
fn run_files(root: &Path, home: &Path) -> TestResult<RunFiles> {
    Ok(UserFolder::in_home(home).run_files(&RepoId::of(&canonical_root(root)?)))
}

/// A hook event of `hook` from session `s1` at `ts_ms`.
fn event(root: &Path, hook: HookKind, ts_ms: i64) -> TestResult<Event> {
    let root = canonical_root(root)?;
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, b"read next"),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("s1")?,
        parent_session_id: None,
        cwd: root.clone(),
        repo_root: root,
        worktree_root: None,
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

/// What the daemon answers `body` with, waiting as long as it takes: the text of its reply.
fn reply(files: &RunFiles, body: RequestBody) -> TestResult<Option<String>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    match ask(files, Role::Hook, body, deadline)?.body {
        ResponseBody::Reply { text, .. } => Ok(text),
        other => Err(format!("{other:?}").into()),
    }
}

/// Starts session `s1`'s task with a prompt at 20 and waits until it is stored: the task.
fn task_started(root: &Path, files: &RunFiles) -> TestResult<Ulid> {
    let prompt = Event {
        prompt_text: Some("fix the orders service".to_owned()),
        ..event(root, HookKind::Prompt, 20)?
    };
    reply(
        files,
        RequestBody::Retrieve {
            event: Box::new(prompt.clone()),
            answer_by_ms: None,
        },
    )?;
    let task = task_id_of_prompt(prompt.event_id, prompt.ts_ms);
    wait_for(|| {
        Ok(Reader::open(&root.join(".cortex"), 0)?
            .queries()
            .task(task)?
            .is_some())
    })?;
    Ok(task)
}

/// A read of `path` by session `s1` at `ts_ms`, at the hook `hook`, of the lines `ranges` (the
/// whole file when empty).
fn read_event(
    root: &Path,
    (hook, path, ts_ms): (HookKind, &str, i64),
    ranges: Vec<LineRange>,
) -> TestResult<Event> {
    Ok(Event {
        tool: Some(ToolKind::Read),
        tool_target: Some(ToolTarget {
            path: Some(RepoPath::new(path)?),
            ranges,
            ..ToolTarget::default()
        }),
        ..event(root, hook, ts_ms)?
    })
}

/// What the read of `path` at `ts_ms` in session `s1` is told before it runs.
fn read(root: &Path, files: &RunFiles, (path, ts_ms): (&str, i64)) -> TestResult<Option<String>> {
    read_by(root, files, (path, ts_ms), None)
}

/// What the read of `path` at `ts_ms` in session `s1` is told, its hook reading the answer until
/// `answer_by_ms` by the wall clock, or as long as it takes.
fn read_by(
    root: &Path,
    files: &RunFiles,
    (path, ts_ms): (&str, i64),
    answer_by_ms: Option<i64>,
) -> TestResult<Option<String>> {
    reply(
        files,
        RequestBody::ReadHint {
            event: Box::new(read_event(
                root,
                (HookKind::PreTool, path, ts_ms),
                Vec::new(),
            )?),
            answer_by_ms,
        },
    )
}

/// Session `s1` reads `path` whole at `ts_ms`, then, once that read is stored, asks what a read of
/// its lines 1 and 2 at `ts_ms + 1` is told.
fn read_twice(
    root: &Path,
    files: &RunFiles,
    (path, ts_ms): (&str, i64),
) -> TestResult<Option<String>> {
    let whole = read_event(root, (HookKind::PostTool, path, ts_ms), Vec::new())?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let body = RequestBody::Event {
        event: Box::new(whole),
    };
    assert_eq!(
        ask(files, Role::Hook, body, deadline)?.body,
        ResponseBody::Ack
    );
    let stored = || -> TestResult<bool> {
        let reader = Reader::open(&root.join(".cortex"), 0)?;
        Ok(reader
            .queries()
            .events_of_session(&Identifier::new("s1")?)?
            .iter()
            .any(|row| {
                row.record.kind == EventKind::Read
                    && row.record.path.as_ref().map(RepoPath::as_str) == Some(path)
            }))
    };
    wait_for(stored)?;
    let lines = vec![LineRange::new(1, 2)?];
    reply(
        files,
        RequestBody::ReadHint {
            event: Box::new(read_event(
                root,
                (HookKind::PreTool, path, ts_ms + 1),
                lines,
            )?),
            answer_by_ms: None,
        },
    )
}

/// The injections of `task` naming a file as the one earlier tasks read next.
fn next_file_lines(root: &Path, task: Ulid) -> TestResult<Vec<InjectionRow>> {
    Ok(Reader::open(&root.join(".cortex"), 0)?
        .queries()
        .injections_of_task(task)?
        .into_iter()
        .filter(|row| {
            row.record
                .files
                .iter()
                .any(|file| file.reason == Some(ReasonKind::ReadNext))
        })
        .collect())
}

/// A read of the service in a task is told the model comes next: by hand, the way's 2.5 faded one
/// task (the prompt's task is number 1) is 2.4875, at least 2 and all the weight leaving the
/// service, taken 3 times. The line is recorded as naming the model, with its text's hash; a
/// second read of the service in the task is told nothing.
#[test]
fn a_read_in_a_task_is_told_once_what_earlier_tasks_read_next() -> TestResult {
    let (repo, home, _daemon, to) = repository("0", &[(SERVICE, MODEL, 3)])?;
    let [model] = to[..] else {
        return Err("one way".into());
    };
    let root = repo.path();
    let files = run_files(root, home.path())?;
    let task = task_started(root, &files)?;
    assert_eq!(read(root, &files, (SERVICE, 30))?.as_deref(), Some(LINE));
    assert_eq!(read(root, &files, (SERVICE, 40))?, None);
    wait_for(|| Ok(!next_file_lines(root, task)?.is_empty()))?;
    let [row] = &next_file_lines(root, task)?[..] else {
        return Err("one line recorded".into());
    };
    assert_eq!(
        row.record.files,
        [InjectedFile {
            node: model,
            spans: Vec::new(),
            reason: Some(ReasonKind::ReadNext),
            shown: Vec::new(),
            content: None,
        }]
    );
    assert_eq!(row.record.text_hash, Some(ContentHash::of(LINE.as_bytes())));
    Ok(())
}

/// Reads of the service answered at once, each from its own connection released together, name the
/// model once between them: one is told the line, the others nothing, before any of their records
/// is stored as after.
#[test]
fn reads_answered_at_once_name_the_file_once() -> TestResult {
    let (repo, home, _daemon, _) = repository("0", &[(SERVICE, MODEL, 3)])?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    task_started(root, &files)?;
    let together = Barrier::new(READS_AT_ONCE);
    let told: Vec<Result<Option<String>, String>> = std::thread::scope(|scope| {
        let reads: Vec<_> = (0..READS_AT_ONCE)
            .map(|at| {
                let (together, files) = (&together, &files);
                scope.spawn(move || {
                    let ts_ms = 30 + i64::try_from(at).map_err(|error| error.to_string())?;
                    together.wait();
                    read(root, files, (SERVICE, ts_ms)).map_err(|error| error.to_string())
                })
            })
            .collect();
        reads
            .into_iter()
            .map(|read| {
                read.join()
                    .unwrap_or_else(|_| Err("a read panicked".to_owned()))
            })
            .collect()
    });
    let told: Vec<Option<String>> = told.into_iter().collect::<Result<_, _>>()?;
    let lines = told
        .iter()
        .filter(|text| text.as_deref() == Some(LINE))
        .count();
    let silent = told.iter().filter(|text| text.is_none()).count();
    assert_eq!((lines, silent), (1, READS_AT_ONCE - 1), "{told:?}");
    Ok(())
}

/// A line worked out after its hook stopped reading (the hook read until 1 ms after 1970 began)
/// reaches no agent: it is recorded as not delivered, without its text's hash, and the file it
/// named is given back, so the next read of the service is told it.
#[test]
fn a_line_too_late_is_told_at_the_next_read() -> TestResult {
    let (repo, home, _daemon, _) = repository("0", &[(SERVICE, MODEL, 3)])?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    let task = task_started(root, &files)?;
    assert_eq!(read_by(root, &files, (SERVICE, 30), Some(1))?, None);
    assert_eq!(read(root, &files, (SERVICE, 40))?.as_deref(), Some(LINE));
    wait_for(|| Ok(next_file_lines(root, task)?.len() == 2))?;
    let hashes: Vec<Option<ContentHash>> = next_file_lines(root, task)?
        .into_iter()
        .map(|row| row.record.text_hash)
        .collect();
    assert_eq!(hashes, [None, Some(ContentHash::of(LINE.as_bytes()))]);
    Ok(())
}

/// A held-out task's read gets nothing.
#[test]
fn a_held_out_task_is_told_nothing_of_what_comes_next() -> TestResult {
    let (repo, home, _daemon, _) = repository("1", &[(SERVICE, MODEL, 3)])?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    task_started(root, &files)?;
    assert_eq!(read(root, &files, (SERVICE, 30))?, None);
    Ok(())
}

/// The read's own hint and the line together stay within the hint's 120 tokens (480 bytes). By
/// hand: the read hint is 83 bytes, the line 72 and its two paths ("[cortex] After reading " 23,
/// ", earlier tasks here went on to read " 37, " (3 times).\n" 12). With paths of 165 and 160
/// bytes the two make 480 bytes, 120 tokens, and both are given; with 165 and 161, 481 bytes and
/// 121 tokens, and only the read's own hint is, though the line alone (398 bytes) would fit.
#[test]
fn the_line_and_the_read_s_own_hint_stay_within_the_cap() -> TestResult {
    let path = |folder: &str, file: &str| format!("src/{folder}/{file}.py");
    let (fits_from, fits_to) = (
        path(&"a".repeat(150), "service"),
        path(&"b".repeat(147), "model"),
    );
    let (over_from, over_to) = (
        path(&"c".repeat(150), "service"),
        path(&"d".repeat(148), "model"),
    );
    let (repo, home, _daemon, _) =
        repository("0", &[(&fits_from, &fits_to, 3), (&over_from, &over_to, 3)])?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    task_started(root, &files)?;
    let fits = format!(
        "{READ_ALREADY}[cortex] After reading {fits_from}, earlier tasks here went on to read \
         {fits_to} (3 times).\n"
    );
    assert_eq!(fits.len(), 480);
    assert_eq!(
        read_twice(root, &files, (&fits_from, 30))?.as_deref(),
        Some(fits.as_str())
    );
    assert_eq!(
        read_twice(root, &files, (&over_from, 40))?.as_deref(),
        Some(READ_ALREADY)
    );
    Ok(())
}

/// A way the store cannot read back (a count below 0) leaves the read's own hint as it is, the
/// failure logged: the session read the service whole, so its read of lines 1 and 2 is told so.
#[test]
fn a_store_that_cannot_say_what_comes_next_leaves_the_read_hint() -> TestResult {
    let (repo, home, _daemon, _) = repository("0", &[(SERVICE, MODEL, -1)])?;
    let root = repo.path();
    let files = run_files(root, home.path())?;
    task_started(root, &files)?;
    assert_eq!(
        read_twice(root, &files, (SERVICE, 30))?.as_deref(),
        Some(READ_ALREADY)
    );
    let log = UserFolder::in_home(home.path())
        .logs_folder()
        .join(format!("daemon-{}.log", RepoId::of(&canonical_root(root)?)));
    wait_for(|| {
        Ok(std::fs::read_to_string(&log)?
            .lines()
            .any(|line| line.contains("WARN") && line.contains("a read's next file failed")))
    })?;
    Ok(())
}
