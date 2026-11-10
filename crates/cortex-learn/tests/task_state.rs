//! What each open task did, kept as its events are recorded and handed out when it ends: steps in
//! order, files given their nodes at the end, a state rebuilt from its rows after a restart, a cut
//! past the row cap, the states dropped for room, and a rolled-back batch.

mod common;

use common::{
    Store, TestResult, event, failed, name, prompt, start, succeeded, target, task_of, tool,
};
use cortex_core::{
    CommandKind, ContentHash, Event, FileContent, HookKind, LineRange, RepoPath, ShellCommand,
    StartSource, ToolKind, ToolResult, ToolTarget,
};
use cortex_learn::{Commands, FileNode, HeldEdit, Recorder, Restore, RunCheck, Step, TaskState};
use cortex_store::NodeId;

/// A read by `session` at `ts_ms` of `path`, the lines `ranges`, or the whole file for none.
fn read(session: &str, ts_ms: i64, path: &str, ranges: &[(u32, u32)]) -> TestResult<Event> {
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (
            ToolKind::Read,
            target(path, ranges)?,
            succeeded(None, Some(100)),
        ),
    )
}

/// An edit by `session` at `ts_ms` of `path` that changed the lines `ranges` and ended as `result`.
fn edit(
    (session, ts_ms): (&str, i64),
    path: &str,
    ranges: &[(u32, u32)],
    result: Option<ToolResult>,
) -> TestResult<Event> {
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (ToolKind::Edit, target(path, ranges)?, result),
    )
}

/// A search by `session` at `ts_ms` of the whole repository that found `hits`.
fn search(session: &str, ts_ms: i64, hits: &[&str]) -> TestResult<Event> {
    let target = ToolTarget {
        path: Some(RepoPath::root()),
        pattern: Some("divide".to_owned()),
        hits: hits
            .iter()
            .map(|path| RepoPath::new(path))
            .collect::<Result<_, _>>()?,
        ..ToolTarget::default()
    };
    tool(
        HookKind::PostTool,
        (session, ts_ms),
        (ToolKind::Grep, target, succeeded(None, Some(80))),
    )
}

/// The shell command `line` run by `session` at `ts_ms`, ended as `result`, in a repository whose
/// root is absolute on every platform, so that the paths a restore names resolve.
fn shell(session: &str, ts_ms: i64, line: &str, result: Option<ToolResult>) -> TestResult<Event> {
    let root = std::env::temp_dir().join("repo");
    let target = ToolTarget {
        command: Some(ShellCommand::cut(line)),
        ..ToolTarget::default()
    };
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

/// A shell command that exited `code`, as Claude Code reports one that failed.
fn exited(code: i32) -> Option<ToolResult> {
    Some(ToolResult {
        exit_code: Some(code),
        is_success: code == 0,
        is_interrupted: false,
        output_bytes: Some(10),
        counts: None,
    })
}

/// The file at `path`, with `node`.
fn file(path: &str, node: Option<NodeId>) -> TestResult<FileNode> {
    Ok(FileNode {
        path: RepoPath::new(path)?,
        node,
    })
}

/// The lines `start` to `end`.
fn lines(start: u32, end: u32) -> TestResult<Option<LineRange>> {
    Ok(Some(LineRange::new(start, end)?))
}

/// The one node `nodes` holds.
fn only(nodes: Vec<NodeId>) -> TestResult<NodeId> {
    Ok(nodes.into_iter().next().ok_or("no node")?)
}

/// The one state `ended` holds.
fn one(ended: Vec<TaskState>) -> TestResult<TaskState> {
    let mut ended = ended.into_iter();
    let state = ended.next().ok_or("no task ended")?;
    assert!(ended.next().is_none(), "more than one task ended");
    Ok(state)
}

/// A task's state holds what it did in order, worked out by hand from the events:
/// - 1,100 a read of `calc/ops.py`, lines 10 to 13 (indexed before the session: node `ops`);
/// - 1,200 a search that found `calc/ops.py` and `tests/test_ops.py`, the second indexed only after
///   the search: both have their nodes once the task ends;
/// - 1,300 an edit of `calc/ops.py`, lines 12 to 14;
/// - 1,350 an edit of `main.py` that failed and 1,360 a read of `README.md` that failed: neither;
/// - 1,400 `cargo check && pytest -q`, exit 0: the tests stand for the line (`pytest`, a test);
/// - 1,500 `git checkout -- calc/ops.py`, exit 0: no check, the restore of `calc/ops.py`;
/// - 1,550 `git restore main.py`, exit 1: no check and no restore, which a failed line does not
///   vouch for;
/// - 1,600 a subagent's read of the whole `main.py`: the parent's task; `main.py` never indexed;
/// - 1,700 an edit of `notes.txt` only the file watcher saw, given to the session at work: no lines;
/// - 1,800 the end of the answer hands the state out.
#[test]
fn a_task_s_state_holds_what_it_did_in_order() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let ops = only(store.index(&["calc/ops.py"])?)?;
    let opening = prompt("s1", 1_000, "fix divide")?;
    store.record(
        &mut recorder,
        &[
            start("s1", 500, StartSource::Startup)?,
            opening.clone(),
            read("s1", 1_100, "calc/ops.py", &[(10, 13)])?,
            search("s1", 1_200, &["calc/ops.py", "tests/test_ops.py"])?,
        ],
    )?;
    let test_ops = only(store.index(&["tests/test_ops.py"])?)?;
    store.record(
        &mut recorder,
        &[
            edit(
                ("s1", 1_300),
                "calc/ops.py",
                &[(12, 14)],
                succeeded(None, None),
            )?,
            edit(("s1", 1_350), "main.py", &[(1, 2)], failed())?,
            tool(
                HookKind::PostTool,
                ("s1", 1_360),
                (ToolKind::Read, target("README.md", &[])?, failed()),
            )?,
            shell("s1", 1_400, "cargo check && pytest -q", exited(0))?,
            shell("s1", 1_500, "git checkout -- calc/ops.py", exited(0))?,
            shell("s1", 1_550, "git restore main.py", exited(1))?,
            Event {
                parent_session_id: Some(name("s1")?),
                ..read("s1/agent-1", 1_600, "main.py", &[])?
            },
        ],
    )?;
    let mut transaction = store.writer.transaction()?;
    let watched = HeldEdit {
        path: RepoPath::new("notes.txt")?,
        seen_ms: 1_700,
        before: FileContent::Absent,
    };
    recorder.record_watcher_edit(&mut transaction, &watched)?;
    transaction.commit()?;
    let state = one(store.record(&mut recorder, &[event(HookKind::Stop, "s1", 1_800)?])?)?;
    assert_eq!(state.task_id(), task_of(&opening)?);
    assert_eq!(
        state.steps(),
        [
            Step::Read {
                at_ms: 1_100,
                file: file("calc/ops.py", Some(ops))?,
                range: lines(10, 13)?,
            },
            Step::Search {
                at_ms: 1_200,
                hits: vec![
                    file("calc/ops.py", Some(ops))?,
                    file("tests/test_ops.py", Some(test_ops))?,
                ],
            },
            Step::Edit {
                at_ms: 1_300,
                file: file("calc/ops.py", Some(ops))?,
                range: lines(12, 14)?,
            },
            Step::Run {
                at_ms: 1_400,
                check: Some(RunCheck {
                    runner: name("pytest")?,
                    kind: CommandKind::Test,
                }),
                exit_code: Some(0),
                restores: Vec::new(),
                counts: None,
            },
            Step::Run {
                at_ms: 1_500,
                check: None,
                exit_code: Some(0),
                restores: vec![Restore::Paths(vec![RepoPath::new("calc/ops.py")?])],
                counts: None,
            },
            Step::Run {
                at_ms: 1_550,
                check: None,
                exit_code: Some(1),
                restores: Vec::new(),
                counts: None,
            },
            Step::Read {
                at_ms: 1_600,
                file: file("main.py", None)?,
                range: None,
            },
            Step::Edit {
                at_ms: 1_700,
                file: file("notes.txt", None)?,
                range: None,
            },
        ]
    );
    assert!(!state.is_partial());
    Ok(())
}

/// After a restart, the state of a task still open is rebuilt from its rows when the task is next
/// touched, without what the rows do not keep: a search's files and a shell line's restores. In
/// session `s1` a read comes first after the restart; in `s2`, the end of the answer itself.
#[test]
fn a_task_open_across_a_restart_is_rebuilt_from_its_rows() -> TestResult {
    let mut store = Store::new()?;
    let ops = only(store.index(&["calc/ops.py"])?)?;
    let mut before = Recorder::new(false, 0.0, Commands::default());
    store.record(
        &mut before,
        &[
            start("s1", 500, StartSource::Startup)?,
            prompt("s1", 1_000, "fix divide")?,
            read("s1", 1_100, "calc/ops.py", &[(10, 13)])?,
            search("s1", 1_200, &["calc/ops.py"])?,
            shell("s1", 1_300, "git checkout -- calc/ops.py", exited(0))?,
            edit(
                ("s1", 1_400),
                "calc/ops.py",
                &[(12, 14)],
                succeeded(None, None),
            )?,
            start("s2", 600, StartSource::Startup)?,
            prompt("s2", 1_050, "read it")?,
            search("s2", 1_150, &["main.py"])?,
        ],
    )?;
    drop(before);
    let mut after = Recorder::new(false, 0.0, Commands::default());
    let ended = store.record(
        &mut after,
        &[
            read("s1", 1_500, "main.py", &[])?,
            event(HookKind::Stop, "s1", 1_600)?,
            event(HookKind::Stop, "s2", 1_700)?,
        ],
    )?;
    let steps: Vec<&[Step]> = ended.iter().map(TaskState::steps).collect();
    assert_eq!(
        steps,
        [
            &[
                Step::Read {
                    at_ms: 1_100,
                    file: file("calc/ops.py", Some(ops))?,
                    range: lines(10, 13)?,
                },
                Step::Search {
                    at_ms: 1_200,
                    hits: Vec::new(),
                },
                Step::Run {
                    at_ms: 1_300,
                    check: None,
                    exit_code: Some(0),
                    restores: Vec::new(),
                    counts: None,
                },
                Step::Edit {
                    at_ms: 1_400,
                    file: file("calc/ops.py", Some(ops))?,
                    range: lines(12, 14)?,
                },
                Step::Read {
                    at_ms: 1_500,
                    file: file("main.py", None)?,
                    range: None,
                },
            ][..],
            &[Step::Search {
                at_ms: 1_150,
                hits: Vec::new(),
            }][..],
        ]
    );
    Ok(())
}

/// A task's state is built from its first 4,096 rows, and a task with more is partial, whether its
/// state was held all along or rebuilt from its rows (a restart before the task ends). Ended by the
/// end of the answer, whose row is the task's last: 4,094 reads fit with the prompt and the end
/// (4,096 rows, all kept); with 4,095 the end is the 4,097th row; with 4,096 the last read is past
/// the cap too, and the steps end at the 4,095th read, at 5,095. Ended by the session's next
/// prompt, whose row is the next task's: 4,095 reads fit with the prompt; with 4,096 the last read
/// is the 4,097th row, which a rebuild must read to tell.
#[test]
fn a_task_past_its_row_cap_is_partial() -> TestResult {
    let cases = [
        (4_094, HookKind::Stop, false, 4_094),
        (4_095, HookKind::Stop, true, 4_095),
        (4_096, HookKind::Stop, true, 4_095),
        (4_095, HookKind::Prompt, false, 4_095),
        (4_096, HookKind::Prompt, true, 4_095),
    ];
    for (reads, end, is_partial, kept) in cases {
        for restarts in [false, true] {
            let case = format!("{reads} reads, ended by {end:?}, restarts {restarts}");
            let mut store = Store::new()?;
            let mut recorder = Recorder::new(false, 0.0, Commands::default());
            store.record(
                &mut recorder,
                &[
                    start("s1", 500, StartSource::Startup)?,
                    prompt("s1", 1_000, "read everything")?,
                ],
            )?;
            let mut transaction = store.writer.transaction()?;
            for at in 1..=reads {
                recorder.record(&mut transaction, &read("s1", 1_000 + at, "a.py", &[])?)?;
            }
            transaction.commit()?;
            if restarts {
                recorder = Recorder::new(false, 0.0, Commands::default());
            }
            let ending = match end {
                HookKind::Prompt => prompt("s1", 9_000, "next")?,
                _ => event(end, "s1", 9_000)?,
            };
            let state = one(store.record(&mut recorder, &[ending])?)?;
            assert_eq!(state.is_partial(), is_partial, "{case}");
            assert_eq!(state.steps().len(), kept, "{case}");
            assert_eq!(
                state.steps().last(),
                Some(&Step::Read {
                    at_ms: 1_000 + i64::try_from(kept)?,
                    file: file("a.py", None)?,
                    range: None,
                }),
                "{case}"
            );
        }
    }
    Ok(())
}

/// At most 32 states are held, and the one touched longest ago is dropped first, not the one made
/// first. Thirty-two sessions each start a task and search for `main.py`; then `s0`'s task searches
/// again, for `notes.md`, so `s1`'s state is the one touched longest ago when the 33rd task starts,
/// and it is dropped. Ending `s1`'s task rebuilds its state, without its search's files (and drops
/// `s2`'s for room); `s0`'s task, held throughout, ends with both searches' files.
#[test]
fn the_state_touched_longest_ago_is_dropped_for_room() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let mut events = Vec::new();
    for number in 0..32 {
        let session = format!("s{number}");
        let at = 1_000 + number * 10;
        events.push(start(&session, at, StartSource::Startup)?);
        events.push(prompt(&session, at + 1, "look")?);
        events.push(search(&session, at + 2, &["main.py"])?);
    }
    events.push(search("s0", 1_500, &["notes.md"])?);
    events.push(start("s32", 1_600, StartSource::Startup)?);
    events.push(prompt("s32", 1_601, "look")?);
    store.record(&mut recorder, &events)?;
    let ended = store.record(
        &mut recorder,
        &[
            event(HookKind::Stop, "s1", 2_000)?,
            event(HookKind::Stop, "s0", 2_001)?,
        ],
    )?;
    let steps: Vec<&[Step]> = ended.iter().map(TaskState::steps).collect();
    assert_eq!(
        steps,
        [
            &[Step::Search {
                at_ms: 1_012,
                hits: Vec::new(),
            }][..],
            &[
                Step::Search {
                    at_ms: 1_002,
                    hits: vec![file("main.py", None)?],
                },
                Step::Search {
                    at_ms: 1_500,
                    hits: vec![file("notes.md", None)?],
                },
            ][..],
        ]
    );
    Ok(())
}

/// A task's searches keep at most 64 KiB of the paths of the files they found, the first found
/// first, and none after a file did not fit. In `s1` a search finds a hundred files of 1,024 bytes:
/// the first 64 are kept, 65,536 bytes, the bound itself. In `s2` a search finds a hundred files of
/// 1,000 bytes: the first 65 are kept (65,000 bytes; a 66th would make 66,000); a later search's
/// `a.py`, which would fit in the 536 bytes left, is not kept either.
#[test]
fn a_task_keeps_its_searches_files_within_their_bound() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let named = |letters: usize| -> Vec<String> {
        (0..100)
            .map(|number| format!("{}{number:02}.py", "a".repeat(letters)))
            .collect()
    };
    let (long, shorter) = (named(1_019), named(995));
    /// The names as text.
    fn as_str(names: &[String]) -> Vec<&str> {
        names.iter().map(String::as_str).collect()
    }
    let ended = store.record(
        &mut recorder,
        &[
            start("s1", 500, StartSource::Startup)?,
            prompt("s1", 1_000, "look")?,
            search("s1", 1_100, &as_str(&long))?,
            event(HookKind::Stop, "s1", 1_200)?,
            start("s2", 2_000, StartSource::Startup)?,
            prompt("s2", 2_100, "look")?,
            search("s2", 2_200, &as_str(&shorter))?,
            search("s2", 2_300, &["a.py"])?,
            event(HookKind::Stop, "s2", 2_400)?,
        ],
    )?;
    let first = |names: &[String], count: usize| -> TestResult<Vec<FileNode>> {
        names
            .get(..count)
            .ok_or("too few names")?
            .iter()
            .map(|path| file(path, None))
            .collect()
    };
    let steps: Vec<&[Step]> = ended.iter().map(TaskState::steps).collect();
    assert_eq!(
        steps,
        [
            &[Step::Search {
                at_ms: 1_100,
                hits: first(&long, 64)?,
            }][..],
            &[
                Step::Search {
                    at_ms: 2_200,
                    hits: first(&shorter, 65)?,
                },
                Step::Search {
                    at_ms: 2_300,
                    hits: Vec::new(),
                },
            ][..],
        ]
    );
    Ok(())
}

/// A stored row that cannot be read back, of a kind this Cortex does not know, leaves a state
/// rebuilt after a restart partial and without the task's earlier steps; the events of the task
/// are still recorded, and the state holds those that came after.
#[test]
fn a_row_that_cannot_be_read_leaves_the_state_partial() -> TestResult {
    let mut store = Store::new()?;
    let mut before = Recorder::new(false, 0.0, Commands::default());
    store.record(
        &mut before,
        &[
            start("s1", 500, StartSource::Startup)?,
            prompt("s1", 1_000, "fix divide")?,
            read("s1", 1_100, "calc/ops.py", &[])?,
        ],
    )?;
    let connection = rusqlite::Connection::open(store.folder().join("cortex.db"))?;
    connection.execute("UPDATE events SET kind = 'reed' WHERE kind = 'read'", [])?;
    drop(connection);
    let mut after = Recorder::new(false, 0.0, Commands::default());
    let state = one(store.record(
        &mut after,
        &[
            read("s1", 1_200, "main.py", &[])?,
            event(HookKind::Stop, "s1", 1_300)?,
        ],
    )?)?;
    assert!(state.is_partial());
    assert_eq!(
        state.steps(),
        [Step::Read {
            at_ms: 1_200,
            file: file("main.py", None)?,
            range: None,
        }]
    );
    Ok(())
}

/// After a transaction is rolled back, the states are rebuilt from what the store holds: the edit
/// of the rolled-back batch is gone, and so are the search's files.
#[test]
fn a_rolled_back_batch_leaves_the_states_to_the_store() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    store.record(
        &mut recorder,
        &[
            start("s1", 500, StartSource::Startup)?,
            prompt("s1", 1_000, "fix divide")?,
            search("s1", 1_100, &["calc/ops.py"])?,
        ],
    )?;
    {
        let mut transaction = store.writer.transaction()?;
        recorder.record(
            &mut transaction,
            &edit(
                ("s1", 1_200),
                "calc/ops.py",
                &[(1, 2)],
                succeeded(None, None),
            )?,
        )?;
    }
    recorder.forget_stored();
    let state = one(store.record(&mut recorder, &[event(HookKind::Stop, "s1", 1_300)?])?)?;
    assert_eq!(
        state.steps(),
        [Step::Search {
            at_ms: 1_100,
            hits: Vec::new(),
        }]
    );
    Ok(())
}

/// Records the edits held by `recorder` due by `now_ms`, in a transaction of their own.
fn record_due(store: &mut Store, recorder: &mut Recorder, now_ms: i64) -> TestResult {
    let mut transaction = store.writer.transaction()?;
    for edit in recorder.take_due_watcher_edits(now_ms) {
        recorder.record_watcher_edit(&mut transaction, &edit)?;
    }
    transaction.commit()?;
    Ok(())
}

/// A task keeps what each file it edited held before it first changed it, whichever came first,
/// the hook's edit or the index's report of the change, worked out by hand:
/// - `a.py` (`a = 1`): the hook's edit at 1,100 comes first, while the store still holds `a = 1`;
///   the index's report, held from 1,300, says the same;
/// - `b.py` (`b = 1`): the index reports the change first, held from 1,400, the store now holding
///   `b = 2`; the hook's edit at 1,500 takes `b = 1` from the held report, not the store;
/// - `c.py` (`c = 1`), changed by no hook (a command, say): the watcher's edit, held from 1,600 and
///   given to `s1` once due, brings `c = 1`;
/// - `d.py`: an edit that failed changed nothing and keeps nothing;
/// - `e.py`, written new at 1,900: no file before;
/// - `a.py` again at 4,000, its report recorded meanwhile, the store holding `a = 2`: the task
///   keeps `a = 1`, what it held before the task first changed it.
#[test]
fn a_task_keeps_what_each_file_held_before_it_changed_it() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    store.index_text(&[
        ("a.py", "a = 1\n"),
        ("b.py", "b = 1\n"),
        ("c.py", "c = 1\n"),
    ])?;
    let hashed = |text: &str| FileContent::Hashed(ContentHash::of(text.as_bytes()));
    store.record(
        &mut recorder,
        &[
            start("s1", 500, StartSource::Startup)?,
            prompt("s1", 1_000, "fix it")?,
            edit(("s1", 1_100), "a.py", &[(1, 1)], succeeded(None, None))?,
        ],
    )?;
    store.index_text(&[("a.py", "a = 2\n"), ("b.py", "b = 2\n")])?;
    recorder.hold_watcher_edits([(RepoPath::new("a.py")?, hashed("a = 1\n"))], 1_300);
    recorder.hold_watcher_edits([(RepoPath::new("b.py")?, hashed("b = 1\n"))], 1_400);
    store.record(
        &mut recorder,
        &[edit(
            ("s1", 1_500),
            "b.py",
            &[(1, 1)],
            succeeded(None, None),
        )?],
    )?;
    store.index_text(&[("c.py", "c = 2\n")])?;
    recorder.hold_watcher_edits([(RepoPath::new("c.py")?, hashed("c = 1\n"))], 1_600);
    store.record(
        &mut recorder,
        &[
            edit(("s1", 1_800), "d.py", &[(1, 1)], failed())?,
            tool(
                HookKind::PostTool,
                ("s1", 1_900),
                (ToolKind::Write, target("e.py", &[])?, succeeded(None, None)),
            )?,
        ],
    )?;
    record_due(&mut store, &mut recorder, 3_800)?;
    let ended = store.record(
        &mut recorder,
        &[
            edit(("s1", 4_000), "a.py", &[(1, 1)], succeeded(None, None))?,
            event(HookKind::Stop, "s1", 5_000)?,
        ],
    )?;
    let before: Vec<(String, FileContent)> = one(ended)?
        .before()
        .iter()
        .map(|(path, content)| (path.to_string(), *content))
        .collect();
    assert_eq!(
        before,
        [
            ("a.py".to_owned(), hashed("a = 1\n")),
            ("b.py".to_owned(), hashed("b = 1\n")),
            ("c.py".to_owned(), hashed("c = 1\n")),
            ("e.py".to_owned(), FileContent::Absent),
        ]
    );
    Ok(())
}

/// What a file held before a task first changed it is left unknown, rather than taken from what
/// tells of an earlier change, when the task's edit comes too close to another's, and is taken
/// when it does not, worked out by hand. Every file starts as read at 1 holding `<name> = 1`.
///
/// `s1`'s first task, from 1,000, edits `a.py` at 1,100, `g.py` at 1,120 and `d.py` at 1,150,
/// writes `e.py` new at 1,180, edits `b.py` at 1,200 and `h.py` at 1,250, and fails to edit
/// `k.py` at 1,260. The index reads `g = 2` at 1,130, `d = 2` and `h = 2` at 1,250, and `a = 2` at
/// 1,300, whose report it holds from then. The task ends at 1,400 having kept what each file held
/// first: `a`, `b`, `d`, `g` and `h` at 1, `e.py` no file.
///
/// The next task, from 1,500:
/// - edits `b.py` at 1,700: the index has not read it since the first task's edit at 1,200, so the
///   store still holds what it held before that edit: unknown;
/// - edits `d.py` at 1,750: read at 1,250, after the edit at 1,150: `d = 2`;
/// - edits `e.py` at 1,780: never read, and written at 1,180: unknown;
/// - edits `c.py` at 1,800, which no task edited: `c = 1`;
/// - edits `h.py` at 1,850: read at 1,250, the very moment of the edit, which may not have been
///   seen: unknown;
/// - edits `k.py` at 1,900: the edit at 1,260 failed and changed nothing: `k = 1`;
/// - edits `a.py` at 3,250, 1,950 after the report's 1,300 and so within its hold, which the edit
///   at 1,100 took as its own, being within 2 s of the report though not of this edit: unknown;
/// - once that report is let go and the index has read `a = 2` at 3,450, and a change no hook made
///   gives `g = 3`, held from 3,700 with `g = 2` before it, edits `g.py` at 3,800: the edit at
///   1,120 is 2,580 before the report, too far to have made it, so the report tells: `g = 2`;
/// - edits `a.py` again at 3,850, the store holding `a = 2` read after every edit of it: too late
///   to say what it held before the task first changed it, at 3,250, so still unknown.
///
/// The next task keeps `c = 1`, `d = 2`, `g = 2` and `k = 1`.
#[test]
fn an_edit_too_close_to_another_leaves_what_its_file_held_unknown() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let names = ["a", "b", "c", "d", "g", "h", "k"];
    let paths: Vec<String> = names.iter().map(|name| format!("{name}.py")).collect();
    let texts: Vec<String> = names.iter().map(|name| format!("{name} = 1\n")).collect();
    let files: Vec<(&str, &str)> = paths
        .iter()
        .zip(&texts)
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    store.index_text(&files)?;
    let hashed = |text: &str| FileContent::Hashed(ContentHash::of(text.as_bytes()));
    let made = || succeeded(None, None);
    let change = |ts_ms: i64, path: &str| edit(("s1", ts_ms), path, &[(1, 1)], made());
    let mut ended = store.record(
        &mut recorder,
        &[
            start("s1", 500, StartSource::Startup)?,
            prompt("s1", 1_000, "fix it")?,
            change(1_100, "a.py")?,
            change(1_120, "g.py")?,
            change(1_150, "d.py")?,
            tool(
                HookKind::PostTool,
                ("s1", 1_180),
                (ToolKind::Write, target("e.py", &[])?, made()),
            )?,
            change(1_200, "b.py")?,
            change(1_250, "h.py")?,
            edit(("s1", 1_260), "k.py", &[(1, 1)], failed())?,
        ],
    )?;
    store.index_text_at(&[("g.py", "g = 2\n")], 1_130)?;
    store.index_text_at(&[("d.py", "d = 2\n"), ("h.py", "h = 2\n")], 1_250)?;
    store.index_text_at(&[("a.py", "a = 2\n")], 1_300)?;
    recorder.hold_watcher_edits([(RepoPath::new("a.py")?, hashed("a = 1\n"))], 1_300);
    ended.extend(store.record(
        &mut recorder,
        &[
            event(HookKind::Stop, "s1", 1_400)?,
            prompt("s1", 1_500, "fix it again")?,
            change(1_700, "b.py")?,
            change(1_750, "d.py")?,
            change(1_780, "e.py")?,
            change(1_800, "c.py")?,
            change(1_850, "h.py")?,
            change(1_900, "k.py")?,
            change(3_250, "a.py")?,
        ],
    )?);
    record_due(&mut store, &mut recorder, 3_400)?;
    store.index_text_at(&[("a.py", "a = 2\n")], 3_450)?;
    store.index_text_at(&[("g.py", "g = 3\n")], 3_700)?;
    recorder.hold_watcher_edits([(RepoPath::new("g.py")?, hashed("g = 2\n"))], 3_700);
    ended.extend(store.record(
        &mut recorder,
        &[
            change(3_800, "g.py")?,
            change(3_850, "a.py")?,
            event(HookKind::Stop, "s1", 4_000)?,
        ],
    )?);
    let before: Vec<Vec<(String, FileContent)>> = ended
        .iter()
        .map(|state| {
            state
                .before()
                .iter()
                .map(|(path, content)| (path.to_string(), *content))
                .collect()
        })
        .collect();
    let kept = |name: &str, number: u32| {
        (
            format!("{name}.py"),
            hashed(&format!("{name} = {number}\n")),
        )
    };
    assert_eq!(
        before,
        [
            vec![
                kept("a", 1),
                kept("b", 1),
                kept("d", 1),
                ("e.py".to_owned(), FileContent::Absent),
                kept("g", 1),
                kept("h", 1),
            ],
            vec![kept("c", 1), kept("d", 2), kept("g", 2), kept("k", 1)],
        ]
    );
    Ok(())
}

/// A task whose state was let go and rebuilt from its rows does not know what the files it edited
/// before held, and a later edit does not tell it: `c.py` (`c = 1`) is edited at 1,100, the
/// recorder forgets its states (as after a batch that could not be recorded), the index reads
/// `c = 2` at 1,300, and the task edits `c.py` again at 5,000. What the store holds then is what
/// the task's first edit made, not what came before it, so the task keeps nothing for `c.py`.
#[test]
fn a_rebuilt_task_takes_no_later_content_as_its_files_earlier_one() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    store.index_text(&[("c.py", "c = 1\n")])?;
    store.record(
        &mut recorder,
        &[
            start("s1", 500, StartSource::Startup)?,
            prompt("s1", 1_000, "fix it")?,
            edit(("s1", 1_100), "c.py", &[(1, 1)], succeeded(None, None))?,
        ],
    )?;
    recorder.forget_stored();
    store.index_text_at(&[("c.py", "c = 2\n")], 1_300)?;
    let ended = store.record(
        &mut recorder,
        &[
            edit(("s1", 5_000), "c.py", &[(1, 1)], succeeded(None, None))?,
            event(HookKind::Stop, "s1", 6_000)?,
        ],
    )?;
    let state = one(ended)?;
    assert_eq!(state.steps().len(), 2);
    assert!(state.before().is_empty(), "{:?}", state.before());
    Ok(())
}

/// While an index runs, a hook's edit takes nothing from a row that index wrote for its file, which
/// may hold the edit's change already, the index's report of it not yet held; a file the index has
/// not written, and any file once the index is done, gives its row. Worked out by hand: the rows
/// read at 1 hold `a = 1`, `b = 1` and `c = 1`. An index that started at 2,000 writes `a = 2` and
/// `c = 2`; meanwhile the task's edits of `a.py` at 2,050 (unknown) and `b.py` at 2,060 (`b = 1`)
/// are recorded. Once the index is done, the task's edit of `c.py` at 2,500 finds `c = 2`, what
/// the file holds before this edit.
#[test]
fn an_edit_recorded_while_an_index_runs_takes_nothing_from_what_it_wrote() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    store.index_text(&[
        ("a.py", "a = 1\n"),
        ("b.py", "b = 1\n"),
        ("c.py", "c = 1\n"),
    ])?;
    let hashed = |text: &str| FileContent::Hashed(ContentHash::of(text.as_bytes()));
    let made = || succeeded(None, None);
    store.record(
        &mut recorder,
        &[
            start("s1", 500, StartSource::Startup)?,
            prompt("s1", 1_000, "fix it")?,
        ],
    )?;
    recorder.index_started(2_000);
    store.index_text_at(&[("a.py", "a = 2\n"), ("c.py", "c = 2\n")], 2_000)?;
    store.record(
        &mut recorder,
        &[
            edit(("s1", 2_050), "a.py", &[(1, 1)], made())?,
            edit(("s1", 2_060), "b.py", &[(1, 1)], made())?,
        ],
    )?;
    recorder.index_done();
    let ended = store.record(
        &mut recorder,
        &[
            edit(("s1", 2_500), "c.py", &[(1, 1)], made())?,
            event(HookKind::Stop, "s1", 3_000)?,
        ],
    )?;
    let before: Vec<(String, FileContent)> = one(ended)?
        .before()
        .iter()
        .map(|(path, content)| (path.to_string(), *content))
        .collect();
    assert_eq!(
        before,
        [
            ("b.py".to_owned(), hashed("b = 1\n")),
            ("c.py".to_owned(), hashed("c = 2\n")),
        ]
    );
    Ok(())
}
