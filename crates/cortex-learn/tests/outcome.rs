//! What became of each task: what the checks after its last edit vouch for, the edits it or the
//! task after it put back, a task left before its answer ended, and a state that shows only part
//! of what the task did.

mod common;

use std::collections::BTreeMap;

use common::{Store, TestResult, end, event, prompt, start, succeeded, target, tool};
use cortex_core::{
    ContentHash, Event, FileContent, HookKind, RepoPath, ShellCommand, StartSource, TaskOutcome,
    ToolKind, ToolResult, ToolTarget,
};
use cortex_learn::{Commands, FileNow, Judgement, Recorder, TaskState, judge, reverted_after};

/// A moment after every step of every task here, when the files are read.
const AFTER_EVERY_STEP: i64 = 1_000_000;

/// One case of a table: the session the task ran in, when the session started, what the task did,
/// and the outcome and words expected of it.
type Case = (&'static str, i64, Vec<Event>, (TaskOutcome, &'static str));

/// One case of a table whose task runs shell command lines: the session, when it started, the
/// lines it ran after its edits, and the outcome and words expected of it.
type Lines = (
    &'static str,
    i64,
    Vec<&'static str>,
    (TaskOutcome, &'static str),
);

/// An edit by `session` at `ts_ms` of `path`, of its first line, that went through.
fn edit(session: &str, ts_ms: i64, path: &str) -> TestResult<Event> {
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (
            ToolKind::Edit,
            target(path, &[(1, 1)])?,
            succeeded(None, None),
        ),
    )
}

/// A read by `session` at `ts_ms` of the whole of `path`.
fn read(session: &str, ts_ms: i64, path: &str) -> TestResult<Event> {
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (
            ToolKind::Read,
            target(path, &[])?,
            succeeded(None, Some(80)),
        ),
    )
}

/// The shell command `line` run by `session` at `ts_ms` in the repository's top folder, which
/// exited `exit_code`, or was interrupted before it finished for none. The root is absolute on
/// every platform, so that the paths a restore names resolve.
fn shell(session: &str, ts_ms: i64, line: &str, exit_code: Option<i32>) -> TestResult<Event> {
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

/// What each file holds, read after every step of every task here.
fn now(files: &[(&str, FileContent)]) -> TestResult<BTreeMap<RepoPath, FileNow>> {
    read_at(AFTER_EVERY_STEP, files)
}

/// What each file holds, read at `read_at_ms`.
fn read_at(
    read_at_ms: i64,
    files: &[(&str, FileContent)],
) -> TestResult<BTreeMap<RepoPath, FileNow>> {
    files
        .iter()
        .map(|(path, content)| {
            let now = FileNow {
                content: *content,
                read_at_ms,
            };
            Ok((RepoPath::new(path)?, now))
        })
        .collect()
}

/// The contents of `text`, hashed as the index hashes them.
fn hashed(text: &str) -> FileContent {
    FileContent::Hashed(ContentHash::of(text.as_bytes()))
}

/// The state of one task: a session of its own started at `at_ms`, a prompt, then `steps`, then
/// the end of the answer a second later.
fn answered_task(
    (store, recorder): (&mut Store, &mut Recorder),
    (session, at_ms): (&str, i64),
    steps: &[Event],
) -> TestResult<TaskState> {
    let mut events = vec![
        start(session, at_ms, StartSource::Startup)?,
        prompt(session, at_ms + 1, "fix it")?,
    ];
    events.extend(steps.iter().cloned());
    events.push(event(HookKind::Stop, session, at_ms + 1_000)?);
    one(store.record(recorder, &events)?)
}

/// The one state `ended` holds.
fn one(ended: Vec<TaskState>) -> TestResult<TaskState> {
    let mut ended = ended.into_iter();
    let state = ended.next().ok_or("no task ended")?;
    assert!(ended.next().is_none(), "more than one task ended");
    Ok(state)
}

/// The two states `ended` holds, in order.
fn two(ended: Vec<TaskState>) -> TestResult<(TaskState, TaskState)> {
    let mut ended = ended.into_iter();
    let first = ended.next().ok_or("no first task ended")?;
    let second = ended.next().ok_or("no second task ended")?;
    assert!(ended.next().is_none(), "more than two tasks ended");
    Ok((first, second))
}

/// What `judgement` says, as (outcome, evidence), for comparing with the words written by hand. No
/// judgement holds empty evidence, so the empty text stands for none.
fn said(judgement: &Judgement) -> (TaskOutcome, &str) {
    (
        judgement.outcome,
        judgement.evidence.as_deref().unwrap_or_default(),
    )
}

/// The checks that ran after a task's last edit decide what became of it, as far as their exit
/// status vouches: the last of them that finished, and only a test, a build or a type check, which
/// is what a passing check can vouch for. Each case is a task of its own, worked out by hand:
/// - `cargo test` exited 0 after an edit, so the tests passed on the task's work;
/// - `cargo test` exited 1 after an edit;
/// - `cargo test` exited 0 before the last edit, so it says nothing about the work;
/// - `cargo test` exited 0 between two edits, which leaves the second edit unchecked;
/// - `ruff check .` exited 0 after the edit: a lint vouches for no work;
/// - `pytest` exited 0 and then `cargo test` was interrupted, which finished nothing, so the last
///   check that finished tells: the tests passed;
/// - an edit with nothing run;
/// - `cargo test` exited 0 with nothing edited: nothing was changed for it to vouch for;
/// - `npm run build` exited 2 after the edit, which the build's own words say;
/// - `tsc --noEmit` exited 0 after the edit, a type check, which the words say in its turn.
#[test]
fn the_checks_after_the_last_edit_decide_the_outcome() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    let cases: [Case; 10] = [
        (
            "s1",
            1_000,
            vec![
                edit("s1", 1_100, "a.py")?,
                shell("s1", 1_200, "cargo test", Some(0))?,
            ],
            (
                TaskOutcome::Success,
                "tests by cargo exited 0 after the last edit",
            ),
        ),
        (
            "s2",
            2_000,
            vec![
                edit("s2", 2_100, "a.py")?,
                shell("s2", 2_200, "cargo test", Some(1))?,
            ],
            (
                TaskOutcome::Failure,
                "tests by cargo exited 1 after the last edit",
            ),
        ),
        (
            "s3",
            3_000,
            vec![
                shell("s3", 3_100, "cargo test", Some(0))?,
                edit("s3", 3_200, "a.py")?,
            ],
            (TaskOutcome::Unknown, ""),
        ),
        (
            "s4",
            4_000,
            vec![
                edit("s4", 4_100, "a.py")?,
                shell("s4", 4_200, "ruff check .", Some(0))?,
            ],
            (TaskOutcome::Unknown, ""),
        ),
        (
            "s5",
            5_000,
            vec![
                edit("s5", 5_100, "a.py")?,
                shell("s5", 5_200, "pytest", Some(0))?,
                shell("s5", 5_300, "cargo test", None)?,
            ],
            (
                TaskOutcome::Success,
                "tests by pytest exited 0 after the last edit",
            ),
        ),
        (
            "s6",
            6_000,
            vec![edit("s6", 6_100, "a.py")?],
            (TaskOutcome::Unknown, ""),
        ),
        (
            "s7",
            7_000,
            vec![shell("s7", 7_100, "cargo test", Some(0))?],
            (TaskOutcome::Unknown, ""),
        ),
        (
            "s10",
            10_000,
            vec![
                edit("s10", 10_100, "a.py")?,
                shell("s10", 10_200, "tsc --noEmit", Some(0))?,
            ],
            (
                TaskOutcome::Success,
                "the type check by tsc exited 0 after the last edit",
            ),
        ),
        (
            "s9",
            9_000,
            vec![
                edit("s9", 9_100, "a.py")?,
                shell("s9", 9_200, "cargo test", Some(0))?,
                edit("s9", 9_300, "a.py")?,
            ],
            (TaskOutcome::Unknown, ""),
        ),
        (
            "s8",
            8_000,
            vec![
                edit("s8", 8_100, "a.py")?,
                shell("s8", 8_200, "npm run build", Some(2))?,
            ],
            (
                TaskOutcome::Failure,
                "the build by npm exited 2 after the last edit",
            ),
        ),
    ];
    for (session, at_ms, steps, expected) in cases {
        let state = answered_task((&mut store, &mut recorder), (session, at_ms), &steps)?;
        assert_eq!(
            said(&judge(&state, &BTreeMap::new())),
            expected,
            "{session}"
        );
    }
    Ok(())
}

/// A task whose every edited file holds again what it held before the task is reverted, and one
/// whose files do not is not, worked out by hand:
/// - `s1` edits `a.py` (`a = 1` before it) and `b.py` (`b = 1` before it), and ends with both
///   holding what they held, so its work is gone;
/// - `s2` edits the same two and leaves `b.py` changed, so the task stands;
/// - `s3` edits `secret.env`, whose contents the index does not read, so nothing says it is back.
#[test]
fn a_task_whose_edited_files_hold_what_they_held_is_reverted() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index_text(&[("a.py", "a = 1\n"), ("b.py", "b = 1\n")])?;
    let steps = |session: &str, at_ms: i64| -> TestResult<Vec<Event>> {
        Ok(vec![
            edit(session, at_ms + 100, "a.py")?,
            edit(session, at_ms + 200, "b.py")?,
        ])
    };
    let back = now(&[("a.py", hashed("a = 1\n")), ("b.py", hashed("b = 1\n"))])?;
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &steps("s1", 1_000)?,
    )?;
    assert_eq!(
        said(&judge(&state, &back)),
        (
            TaskOutcome::Reverted,
            "every edited file is back as it was before the task"
        )
    );
    let one_changed = now(&[("a.py", hashed("a = 1\n")), ("b.py", hashed("b = 2\n"))])?;
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s2", 2_000),
        &steps("s2", 2_000)?,
    )?;
    assert_eq!(
        said(&judge(&state, &one_changed)),
        (TaskOutcome::Unknown, "")
    );
    store.index(&["secret.env"])?;
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s3", 3_000),
        &[edit("s3", 3_100, "secret.env")?],
    )?;
    let unread = now(&[("secret.env", FileContent::Unread)])?;
    assert_eq!(said(&judge(&state, &unread)), (TaskOutcome::Unknown, ""));
    Ok(())
}

/// A file the task created and that is gone again is back as it was, while one the task deleted is
/// not: `s1` writes `new.py`, which held nothing before it and holds nothing after; `s2` deletes
/// `a.py`, which held `a = 1`.
#[test]
fn a_created_file_that_is_gone_again_is_back() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index_text(&[("a.py", "a = 1\n")])?;
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &[edit("s1", 1_100, "new.py")?],
    )?;
    let gone = now(&[("new.py", FileContent::Absent)])?;
    assert_eq!(
        said(&judge(&state, &gone)),
        (
            TaskOutcome::Reverted,
            "every edited file is back as it was before the task"
        )
    );
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s2", 2_000),
        &[edit("s2", 2_100, "a.py")?],
    )?;
    let deleted = now(&[("a.py", FileContent::Absent)])?;
    assert_eq!(said(&judge(&state, &deleted)), (TaskOutcome::Unknown, ""));
    Ok(())
}

/// A git restore that ran after a file's last edit reverts the task, whatever the index holds for
/// the file, which it may not have read again yet. Worked out by hand, each a task of its own,
/// with the index still holding the edited contents:
/// - `s1`: an edit of `a.py`, then `git restore a.py` exited 0;
/// - `s2`: an edit, then `git reset --hard`, which puts every file back;
/// - `s3`: an edit of `src/a.py`, then `git checkout -- src`, the folder holding it;
/// - `s4`: `git restore a.py` before the edit, which the edit then undid;
/// - `s5`: an edit, then `git restore a.py` exited 1, which restored nothing;
/// - `s6`: an edit of `a.py` and of `b.py`, then a restore of `a.py` alone.
#[test]
fn a_git_restore_after_the_last_edit_reverts_the_task() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index_text(&[
        ("a.py", "a = 1\n"),
        ("b.py", "b = 1\n"),
        ("src/a.py", "a = 1\n"),
    ])?;
    let restored = (
        TaskOutcome::Reverted,
        "every edited file was put back or restored by git",
    );
    let cases: [Case; 6] = [
        (
            "s1",
            1_000,
            vec![
                edit("s1", 1_100, "a.py")?,
                shell("s1", 1_200, "git restore a.py", Some(0))?,
            ],
            restored,
        ),
        (
            "s2",
            2_000,
            vec![
                edit("s2", 2_100, "a.py")?,
                shell("s2", 2_200, "git reset --hard", Some(0))?,
            ],
            restored,
        ),
        (
            "s3",
            3_000,
            vec![
                edit("s3", 3_100, "src/a.py")?,
                shell("s3", 3_200, "git checkout -- src", Some(0))?,
            ],
            restored,
        ),
        (
            "s4",
            4_000,
            vec![
                shell("s4", 4_100, "git restore a.py", Some(0))?,
                edit("s4", 4_200, "a.py")?,
            ],
            (TaskOutcome::Unknown, ""),
        ),
        (
            "s5",
            5_000,
            vec![
                edit("s5", 5_100, "a.py")?,
                shell("s5", 5_200, "git restore a.py", Some(1))?,
            ],
            (TaskOutcome::Unknown, ""),
        ),
        (
            "s6",
            6_000,
            vec![
                edit("s6", 6_100, "a.py")?,
                edit("s6", 6_150, "b.py")?,
                shell("s6", 6_200, "git restore a.py", Some(0))?,
            ],
            (TaskOutcome::Unknown, ""),
        ),
    ];
    let changed = now(&[
        ("a.py", hashed("a = 2\n")),
        ("b.py", hashed("b = 2\n")),
        ("src/a.py", hashed("a = 2\n")),
    ])?;
    for (session, at_ms, steps, expected) in cases {
        let state = answered_task((&mut store, &mut recorder), (session, at_ms), &steps)?;
        assert_eq!(said(&judge(&state, &changed)), expected, "{session}");
    }
    Ok(())
}

/// A task left before its answer ended was abandoned, unless a check after its last edit says how
/// its work went. Worked out by hand: `s1` edits and the next prompt arrives with no end of the
/// answer, so the first task was left; `s2` edits and its session ends; `s3` edits, its tests
/// pass, and then its session ends, which the tests still vouch for.
#[test]
fn a_task_left_before_its_answer_ended_was_abandoned() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    let left = (
        TaskOutcome::Abandoned,
        "the task was left before its answer ended",
    );
    let next_prompt = store.record(
        &mut recorder,
        &[
            start("s1", 1_000, StartSource::Startup)?,
            prompt("s1", 1_100, "fix it")?,
            edit("s1", 1_200, "a.py")?,
            prompt("s1", 1_300, "fix it again")?,
        ],
    )?;
    assert_eq!(said(&judge(&one(next_prompt)?, &BTreeMap::new())), left);
    let session_ended = store.record(
        &mut recorder,
        &[
            start("s2", 2_000, StartSource::Startup)?,
            prompt("s2", 2_100, "fix it")?,
            edit("s2", 2_200, "a.py")?,
            end("s2", 2_300, "clear")?,
        ],
    )?;
    assert_eq!(said(&judge(&one(session_ended)?, &BTreeMap::new())), left);
    let checked = store.record(
        &mut recorder,
        &[
            start("s3", 3_000, StartSource::Startup)?,
            prompt("s3", 3_100, "fix it")?,
            edit("s3", 3_200, "a.py")?,
            shell("s3", 3_300, "cargo test", Some(0))?,
            end("s3", 3_400, "clear")?,
        ],
    )?;
    assert_eq!(
        said(&judge(&one(checked)?, &BTreeMap::new())),
        (
            TaskOutcome::Success,
            "tests by cargo exited 0 after the last edit"
        )
    );
    Ok(())
}

/// A state that shows only part of what its task did tells nothing: the rows it does not hold may
/// hold the last edit, or a check that failed after the checks it does hold. The task here edits
/// `a.py` and sees `cargo test` pass, which would say its tests passed, and then reads its way
/// past the cap: with the prompt and those two, 4,093 reads make 4,096 rows, and the end of the
/// answer is the 4,097th, which leaves the state partial.
#[test]
fn a_task_whose_state_is_partial_has_no_outcome() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.record(
        &mut recorder,
        &[
            start("s1", 1_000, StartSource::Startup)?,
            prompt("s1", 1_001, "fix it")?,
            edit("s1", 1_002, "a.py")?,
            shell("s1", 1_003, "cargo test", Some(0))?,
        ],
    )?;
    let mut transaction = store.writer.transaction()?;
    for at in 1..=4_093 {
        recorder.record(
            &mut transaction,
            &read("s1", 1_003 + i64::from(at), "a.py")?,
        )?;
    }
    transaction.commit()?;
    let state = one(store.record(&mut recorder, &[event(HookKind::Stop, "s1", 9_000)?])?)?;
    assert!(state.is_partial(), "the state shows only part");
    assert_eq!(
        state.steps().len(),
        4_095,
        "the edit, the check and the reads"
    );
    assert_eq!(
        said(&judge(&state, &BTreeMap::new())),
        (TaskOutcome::Unknown, "")
    );
    // Nor can the task after it show its work is gone: the files of the rows it does not hold are
    // not in it to be looked for.
    let after = one(store.record(
        &mut recorder,
        &[
            prompt("s1", 9_100, "undo it")?,
            edit("s1", 9_200, "a.py")?,
            event(HookKind::Stop, "s1", 9_300)?,
        ],
    )?)?;
    let gone = now(&[("a.py", FileContent::Absent)])?;
    assert!(
        reverted_after(&state, &after, &gone).is_none(),
        "a partial state shows no revert"
    );
    Ok(())
}

/// A git restore of a file the task edited, run after a check that passed, leaves the check
/// vouching for work that is no longer there. Worked out by hand, each task editing `a.py` (`a = 1`
/// before it) and `b.py` (`b = 1` before it), so that putting `a.py` back does not revert the task,
/// and the index still holding the edited contents of both:
/// - `s1`: both edits, `cargo test` exited 0, then `git restore a.py`, so the tests passed on
///   work that was then undone;
/// - `s2`: both edits, then `cargo test && git restore a.py` as one line, whose two halves are not
///   ordered in the step that records them;
/// - `s3`: both edits, `git restore a.py`, then `cargo test` exited 0, which did run on what the
///   task left behind;
/// - `s4`: both edits, `cargo test` exited 0, then a restore of `notes.txt`, which the task never
///   edited and which leaves the tests vouching.
#[test]
fn a_restore_after_a_check_leaves_it_vouching_for_nothing() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index_text(&[("a.py", "a = 1\n"), ("b.py", "b = 1\n")])?;
    let both = |session: &str, at_ms: i64| -> TestResult<Vec<Event>> {
        Ok(vec![
            edit(session, at_ms + 100, "a.py")?,
            edit(session, at_ms + 150, "b.py")?,
        ])
    };
    let passed = (
        TaskOutcome::Success,
        "tests by cargo exited 0 after the last edit",
    );
    let lines: [Lines; 4] = [
        (
            "s1",
            1_000,
            vec!["cargo test", "git restore a.py"],
            (TaskOutcome::Unknown, ""),
        ),
        (
            "s2",
            2_000,
            vec!["cargo test && git restore a.py"],
            (TaskOutcome::Unknown, ""),
        ),
        ("s3", 3_000, vec!["git restore a.py", "cargo test"], passed),
        (
            "s4",
            4_000,
            vec!["cargo test", "git restore notes.txt"],
            passed,
        ),
    ];
    let changed = now(&[("a.py", hashed("a = 2\n")), ("b.py", hashed("b = 2\n"))])?;
    for (session, at_ms, commands, expected) in lines {
        let mut steps = both(session, at_ms)?;
        for (number, line) in commands.iter().enumerate() {
            let at_ms = at_ms + 200 + i64::try_from(number)? * 10;
            steps.push(shell(session, at_ms, line, Some(0))?);
        }
        let state = answered_task((&mut store, &mut recorder), (session, at_ms), &steps)?;
        assert_eq!(said(&judge(&state, &changed)), expected, "{session}");
    }
    Ok(())
}

/// A task whose work is gone is reverted however well its checks ran: the tests passed on an edit
/// the task then put back itself. `a.py` held `a = 1` before the task, which edits it, sees
/// `cargo test` exit 0, and edits it back to `a = 1`.
#[test]
fn a_revert_outranks_a_passing_check() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index_text(&[("a.py", "a = 1\n")])?;
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &[
            edit("s1", 1_100, "a.py")?,
            shell("s1", 1_200, "cargo test", Some(0))?,
            edit("s1", 1_300, "a.py")?,
        ],
    )?;
    let back = now(&[("a.py", hashed("a = 1\n"))])?;
    assert_eq!(
        said(&judge(&state, &back)),
        (
            TaskOutcome::Reverted,
            "every edited file is back as it was before the task"
        )
    );
    Ok(())
}

/// What a file holds shows it is back only when it was read after the task last edited it. Until
/// the index reads a change, the store holds what the file held before it, which is the very
/// content the task is compared against, so reading the store again proves nothing. The task edits
/// `a.py` at 1,100, which held `a = 1`: the same `a = 1` read at 1,100 and before it says nothing,
/// and read at 1,101 says the file is back.
#[test]
fn a_content_read_before_the_last_edit_shows_nothing() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index_text(&[("a.py", "a = 1\n")])?;
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &[edit("s1", 1_100, "a.py")?],
    )?;
    for read_at_ms in [1_000, 1_100] {
        let stale = read_at(read_at_ms, &[("a.py", hashed("a = 1\n"))])?;
        assert_eq!(
            said(&judge(&state, &stale)),
            (TaskOutcome::Unknown, ""),
            "read at {read_at_ms}"
        );
    }
    let fresh = read_at(1_101, &[("a.py", hashed("a = 1\n"))])?;
    assert_eq!(
        said(&judge(&state, &fresh)),
        (
            TaskOutcome::Reverted,
            "every edited file is back as it was before the task"
        )
    );
    Ok(())
}

/// A revert can be part what the files hold and part a git restore, and is no revert when nothing
/// says a file is back. Worked out by hand, each task editing `a.py` (`a = 1` before it) and
/// `b.py` (`b = 1` before it) and then running `git restore b.py`, the index still holding
/// `b = 2`: for `s1`, `a.py` is read as `a = 1` again; for `s2`, nothing is read for `a.py`.
#[test]
fn a_revert_can_be_part_contents_and_part_restore() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index_text(&[("a.py", "a = 1\n"), ("b.py", "b = 1\n")])?;
    let steps = |session: &str, at_ms: i64| -> TestResult<Vec<Event>> {
        Ok(vec![
            edit(session, at_ms + 100, "a.py")?,
            edit(session, at_ms + 150, "b.py")?,
            shell(session, at_ms + 200, "git restore b.py", Some(0))?,
        ])
    };
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &steps("s1", 1_000)?,
    )?;
    let part_back = now(&[("a.py", hashed("a = 1\n")), ("b.py", hashed("b = 2\n"))])?;
    assert_eq!(
        said(&judge(&state, &part_back)),
        (
            TaskOutcome::Reverted,
            "every edited file was put back or restored by git"
        )
    );
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s2", 2_000),
        &steps("s2", 2_000)?,
    )?;
    let unread = now(&[("b.py", hashed("b = 2\n"))])?;
    assert_eq!(said(&judge(&state, &unread)), (TaskOutcome::Unknown, ""));
    Ok(())
}

/// The task after a task can show that its work is gone: the files it edited hold again what they
/// held before it, or git put them back while the next task ran. Worked out by hand, each pair of
/// tasks in a session of its own, the first editing `a.py`, which held `a = 1`:
/// - `s1`: the task after it edits `a.py` back to `a = 1`;
/// - `s2`: the task after it runs `git restore a.py`, the index still holding `a = 2`;
/// - `s3`: the task after it edits `b.py` and leaves `a.py` at `a = 2`, so the work stands.
///
/// A restore of the task after it counts only when it ran after the edit it would undo, so a pair
/// that is not a task and the task after it shows nothing the steps do not.
#[test]
fn the_task_after_a_task_can_show_its_work_is_gone() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, Commands::default());
    store.index_text(&[("a.py", "a = 1\n")])?;
    let pair = |session: &str, at_ms: i64, second: Vec<Event>| -> TestResult<Vec<Event>> {
        let mut events = vec![
            start(session, at_ms, StartSource::Startup)?,
            prompt(session, at_ms + 100, "fix it")?,
            edit(session, at_ms + 200, "a.py")?,
            event(HookKind::Stop, session, at_ms + 300)?,
            prompt(session, at_ms + 400, "undo it")?,
        ];
        events.extend(second);
        events.push(event(HookKind::Stop, session, at_ms + 900)?);
        Ok(events)
    };
    let back = now(&[("a.py", hashed("a = 1\n"))])?;
    let changed = now(&[("a.py", hashed("a = 2\n"))])?;
    let ended = store.record(
        &mut recorder,
        &pair("s1", 1_000, vec![edit("s1", 1_500, "a.py")?])?,
    )?;
    let (first, second) = two(ended)?;
    assert_eq!(
        said(&reverted_after(&first, &second, &back).ok_or("no revert")?),
        (
            TaskOutcome::Reverted,
            "every edited file is back as it was before the task, by the end of the next task"
        )
    );
    let ended = store.record(
        &mut recorder,
        &pair(
            "s2",
            2_000,
            vec![shell("s2", 2_500, "git restore a.py", Some(0))?],
        )?,
    )?;
    let (first, second) = two(ended)?;
    assert_eq!(
        said(&reverted_after(&first, &second, &changed).ok_or("no revert")?),
        (
            TaskOutcome::Reverted,
            "every edited file was put back or restored by git, by the end of the next task"
        )
    );
    let ended = store.record(
        &mut recorder,
        &pair("s3", 3_000, vec![edit("s3", 3_500, "b.py")?])?,
    )?;
    let (first, second) = two(ended)?;
    assert!(
        reverted_after(&first, &second, &changed).is_none(),
        "the first task's work stands"
    );
    // A task paired with itself shows no revert its own steps did not: `s4` restores `a.py` and
    // then edits it, and that restore comes before the edit by step and by time alike.
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s4", 4_000),
        &[
            shell("s4", 4_100, "git restore a.py", Some(0))?,
            edit("s4", 4_200, "a.py")?,
        ],
    )?;
    assert!(
        reverted_after(&state, &state, &changed).is_none(),
        "a task is not reverted by what it did before its own edit"
    );
    Ok(())
}
