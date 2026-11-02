//! One record of what a task did: a worked example of every part, the bound its parts are trimmed
//! to, and that nothing of the work's own text reaches it.

mod common;

use common::{Store, TestResult, event, prompt, start, succeeded, target, tool};
use cortex_core::{
    CommandKind, Event, HookKind, Identifier, ShellCommand, StartSource, TaskOutcome, ToolKind,
    ToolResult, ToolTarget,
};
use cortex_learn::{Commands, Recorder, TaskState, episode_of};
use cortex_store::{EpisodeRecord, MAX_EPISODE_BYTES, NodeId};

/// A read by `session` at `ts_ms` of `path`, the lines `range`, or of the whole file for none.
fn read(session: &str, ts_ms: i64, path: &str, range: &[(u32, u32)]) -> TestResult<Event> {
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
fn edit(session: &str, ts_ms: i64, path: &str, range: &[(u32, u32)]) -> TestResult<Event> {
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (ToolKind::Edit, target(path, range)?, succeeded(None, None)),
    )
}

/// The shell command `line` run by `session` at `ts_ms`, which exited `exit_code`, in a repository
/// whose root is absolute on every platform so that the paths a restore names resolve.
fn shell(session: &str, ts_ms: i64, line: &str, exit_code: i32) -> TestResult<Event> {
    let root = std::env::temp_dir().join("repo");
    let target = ToolTarget {
        command: Some(ShellCommand::cut(line)),
        ..ToolTarget::default()
    };
    let result = Some(ToolResult {
        exit_code: Some(exit_code),
        is_success: exit_code == 0,
        is_interrupted: false,
        output_bytes: Some(20),
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

/// The state of one task: a session started at `at_ms`, a prompt of `text`, then `steps`, then the
/// end of the answer.
fn answered_task(
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

/// A name.
fn name(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
}

/// What the record says the task read, as (node, reads).
fn localized(record: &EpisodeRecord) -> Vec<(NodeId, u32)> {
    record
        .localize
        .iter()
        .map(|read| (read.node, read.reads))
        .collect()
}

/// One file the record says the task changed: its node, the lines the changes covered, and the
/// symbols among them with whether a change covered each one's declaration line.
type Changed = (NodeId, u32, Vec<(NodeId, bool)>);

/// What the record says the task changed.
fn changed(record: &EpisodeRecord) -> Vec<Changed> {
    record
        .change
        .iter()
        .map(|file| {
            (
                file.node,
                file.lines,
                file.symbols
                    .iter()
                    .map(|symbol| (symbol.node, symbol.header))
                    .collect(),
            )
        })
        .collect()
}

/// What the record says the task ran, as (runner, kind, code).
fn verified(record: &EpisodeRecord) -> Vec<(String, CommandKind, Option<i32>)> {
    record
        .verify
        .iter()
        .map(|check| (check.runner.as_str().to_owned(), check.kind, check.code))
        .collect()
}

/// One record of what a task did, worked out by hand. `calc/ops.py` holds the functions `add` over
/// lines 1 to 5 and `mul` over 10 to 15; `calc/util.py` holds none. The task, in order:
///
/// - reads `main.py` whole, twice, and `calc/ops.py` lines 1 to 5 once, all before it changes
///   anything, so where it looked is `main.py` twice and `calc/ops.py` once, in that order;
/// - runs `git restore calc/ops.py`, which exits 0 before the file was changed at all, so it put
///   nothing of the task's own work back;
/// - changes `calc/ops.py` lines 1 to 2, which only `add` covers and which include the line its
///   declaration starts on, so `add` is changed with its header;
/// - reads `main.py` whole again, which is after the first change and so is not where it looked;
/// - runs `cargo test`, which exits 1;
/// - changes `calc/ops.py` lines 12 to 13, which only `mul` covers and which do not include line
///   10, where its declaration starts, so `mul` is changed without its header;
/// - changes `calc/ops.py` lines 2 to 3, which line 2 already counted, so the file's changed lines
///   are 1, 2, 3, 12 and 13, which is five and not the six the three changes cover between them;
/// - changes `calc/util.py` line 1, which no symbol covers;
/// - runs `git restore calc/util.py`, which exits 0, so that file was changed and put back;
/// - runs `cargo test` again, which exits 0, so `cargo` is a runner whose check failed and later
///   passed;
/// - runs `echo hi`, which is no check Cortex knows and is not kept;
/// - runs `cargo test` once more and is interrupted, so that check finished with no code.
///
/// Reading `main.py` three times is no pitfall: a read of a whole file names no stretch, and only
/// a stretch read more than three times is one.
#[test]
fn a_record_says_what_one_task_did() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    let ops = store.index_symbols("calc/ops.py", &[("add", 1, 5), ("mul", 10, 15)])?;
    let (ops_file, add, mul) = (ops[0], ops[1], ops[2]);
    let util = store.index_symbols("calc/util.py", &[])?[0];
    let main = store.index(&["main.py"])?[0];
    let interrupted = Some(ToolResult {
        exit_code: None,
        is_success: false,
        is_interrupted: true,
        output_bytes: None,
    });
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        "fix the divide",
        &[
            read("s1", 1_100, "main.py", &[])?,
            read("s1", 1_150, "main.py", &[])?,
            read("s1", 1_200, "calc/ops.py", &[(1, 5)])?,
            shell("s1", 1_250, "git restore calc/ops.py", 0)?,
            edit("s1", 1_300, "calc/ops.py", &[(1, 2)])?,
            read("s1", 1_350, "main.py", &[])?,
            shell("s1", 1_400, "cargo test", 1)?,
            edit("s1", 1_500, "calc/ops.py", &[(12, 13)])?,
            edit("s1", 1_550, "calc/ops.py", &[(2, 3)])?,
            edit("s1", 1_600, "calc/util.py", &[(1, 1)])?,
            shell("s1", 1_700, "git restore calc/util.py", 0)?,
            shell("s1", 1_800, "cargo test", 0)?,
            shell("s1", 1_850, "echo hi", 0)?,
            Event {
                tool_result: interrupted,
                ..shell("s1", 1_900, "cargo test", 0)?
            },
        ],
    )?;
    let record = episode_of(
        store.writer.queries(),
        &state,
        (TaskOutcome::Success, 9_000),
    )?;
    assert_eq!(record.task_id, state.task_id());
    assert_eq!(record.outcome, TaskOutcome::Success);
    assert_eq!(record.created_at_ms, 9_000);
    assert_eq!(localized(&record), [(main, 2), (ops_file, 1)]);
    assert_eq!(
        changed(&record),
        [
            (ops_file, 5, vec![(add, true), (mul, false)]),
            (util, 1, vec![]),
        ]
    );
    assert_eq!(
        verified(&record),
        [
            ("cargo".to_owned(), CommandKind::Test, Some(1)),
            ("cargo".to_owned(), CommandKind::Test, Some(0)),
            ("cargo".to_owned(), CommandKind::Test, None),
        ]
    );
    assert_eq!(record.pitfalls.fixed, [name("cargo")?]);
    assert_eq!(record.pitfalls.undone, [util]);
    assert_eq!(record.pitfalls.read_again, []);
    Ok(())
}

/// A stretch read more than three times is a pitfall, and three reads of it are not: the task
/// reads lines 10 to 20 of `a.py` four times and lines 1 to 2 three times.
#[test]
fn a_stretch_read_over_and_over_is_a_pitfall() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    let a = store.index(&["a.py"])?[0];
    let mut steps = Vec::new();
    for at in 0..4 {
        steps.push(read("s1", 1_100 + at, "a.py", &[(10, 20)])?);
    }
    for at in 0..3 {
        steps.push(read("s1", 1_200 + at, "a.py", &[(1, 2)])?);
    }
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        "read it again",
        &steps,
    )?;
    let record = episode_of(
        store.writer.queries(),
        &state,
        (TaskOutcome::Unknown, 9_000),
    )?;
    let again: Vec<(NodeId, u32, u32, u32)> = record
        .pitfalls
        .read_again
        .iter()
        .map(|read| (read.node, read.from_line, read.to_line, read.times))
        .collect();
    assert_eq!(again, [(a, 10, 20, 4)]);
    Ok(())
}

/// A task of hundreds of changes is trimmed until its parts fit the bound, the pitfalls going
/// first and where it looked last: a task that reads 30 files, changes 300 and runs 30 checks
/// keeps its 20 places looked at, which is all a record keeps of them, and loses enough of the
/// rest to fit.
#[test]
fn a_record_of_a_large_task_is_trimmed_to_fit() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    let read_paths: Vec<String> = (0..30).map(|at| format!("r{at:03}.py")).collect();
    let changed_paths: Vec<String> = (0..300).map(|at| format!("c{at:03}.py")).collect();
    store.index(&read_paths.iter().map(String::as_str).collect::<Vec<&str>>())?;
    store.index(
        &changed_paths
            .iter()
            .map(String::as_str)
            .collect::<Vec<&str>>(),
    )?;
    let mut steps = Vec::new();
    for (at, path) in read_paths.iter().enumerate() {
        steps.push(read("s1", 1_100 + i64::try_from(at)?, path, &[(1, 4)])?);
    }
    for (at, path) in changed_paths.iter().enumerate() {
        steps.push(edit("s1", 2_000 + i64::try_from(at)?, path, &[(1, 2)])?);
    }
    for at in 0..30 {
        steps.push(shell("s1", 3_000 + at, "cargo test", 0)?);
    }
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        "change everything",
        &steps,
    )?;
    let record = episode_of(
        store.writer.queries(),
        &state,
        (TaskOutcome::Success, 9_000),
    )?;
    assert!(
        record.size_bytes()? <= MAX_EPISODE_BYTES,
        "the record should fit, holds {}",
        record.size_bytes()?
    );
    assert_eq!(record.localize.len(), 20, "every place a record keeps");
    assert!(record.verify.is_empty(), "the checks go before the places");
    assert!(!record.change.is_empty(), "what changed is kept in part");
    let mut transaction = store.writer.transaction()?;
    transaction.replace_episode(&record)?;
    transaction.commit()?;
    assert_eq!(
        store
            .writer
            .queries()
            .episode_of_task(state.task_id())?
            .map(|held| held.localize.len()),
        Some(20)
    );
    Ok(())
}

/// A record holds nothing of the work's own text: the prompt, the command line and the search
/// pattern each carry a word of their own, and none of them is anywhere in the record's parts.
#[test]
fn a_record_holds_no_text_of_the_work() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index(&["a.py"])?;
    let searched = ToolTarget {
        pattern: Some("patternmarker".to_owned()),
        ..target("a.py", &[])?
    };
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        "promptmarker",
        &[
            tool(
                HookKind::PostTool,
                ("s1", 1_100),
                (ToolKind::Grep, searched, succeeded(None, Some(10))),
            )?,
            read("s1", 1_200, "a.py", &[(1, 4)])?,
            edit("s1", 1_300, "a.py", &[(1, 2)])?,
            shell("s1", 1_400, "cargo test --features commandmarker", 0)?,
        ],
    )?;
    let record = episode_of(
        store.writer.queries(),
        &state,
        (TaskOutcome::Success, 9_000),
    )?;
    let parts = record.parts()?.join(" ");
    for marker in ["promptmarker", "commandmarker", "patternmarker", "a.py"] {
        assert!(
            !parts.contains(marker),
            "the record holds {marker}: {parts}"
        );
    }
    Ok(())
}
