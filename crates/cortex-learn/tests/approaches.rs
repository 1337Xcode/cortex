//! The approaches a task tried that did not hold: a failure that stayed the same over the task's
//! changes until it changed another file, and the files it changed back. Each worked by hand from
//! the steps a scripted task takes.

mod common;

use std::collections::BTreeMap;

use common::{Store, TestResult, answered_task, edit, event, name, prompt, shell, start};
use cortex_core::{
    ContentHash, Event, FileContent, HookKind, RepoPath, ShortHash, StartSource, TaskOutcome,
};
use cortex_learn::{Commands, FileNow, Recorder, changed_back, episode_of, persisted_failures};
use cortex_store::{NodeId, PersistedFailure};

/// The shell line `line` run by `session` at `ts_ms` that exited `exit_code`, its output naming
/// the failing tests whose signature is `failing`, as the daemon reads it.
fn check(
    (session, ts_ms): (&str, i64),
    line: &str,
    exit_code: i32,
    failing: Option<&str>,
) -> TestResult<Event> {
    let mut run = shell(session, ts_ms, line, Some(exit_code))?;
    if let Some(result) = run.tool_result.as_mut() {
        result.failing = failing.map(|names| ShortHash::of(names.as_bytes()));
    }
    Ok(run)
}

/// A `pytest` run by `session` at `ts_ms`, as [`check`] runs it.
fn pytest(session: &str, ts_ms: i64, exit_code: i32, failing: Option<&str>) -> TestResult<Event> {
    check((session, ts_ms), "pytest", exit_code, failing)
}

/// The persisted failures of one answered task of `session` taking `steps`.
fn persisted(
    store: &mut Store,
    session: &str,
    steps: &[Event],
) -> TestResult<Vec<PersistedFailure>> {
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let state = answered_task((store, &mut recorder), (session, 1_000), steps)?;
    Ok(persisted_failures(&state))
}

/// The task edits `a.py`, pytest fails naming test X, it edits `a.py` again and pytest fails
/// naming X again, so the failure outlasted a change; it edits `b.py`, new to the task, and
/// pytest passes. Changing `a.py` alone left X failing until `b.py` changed too: one persisted
/// failure, pytest's, `a.py` edited and `b.py` the file it took.
#[test]
fn a_failure_that_stayed_until_another_file_changed_is_found() -> TestResult {
    let mut store = Store::new()?;
    let nodes: [NodeId; 2] = store
        .index(&["a.py", "b.py"])?
        .try_into()
        .map_err(|_| "two nodes")?;
    let found = persisted(
        &mut store,
        "s1",
        &[
            edit("s1", 1_100, "a.py", &[(1, 1)])?,
            pytest("s1", 1_200, 1, Some("X"))?,
            edit("s1", 1_300, "a.py", &[(2, 2)])?,
            pytest("s1", 1_400, 1, Some("X"))?,
            edit("s1", 1_500, "b.py", &[(1, 1)])?,
            pytest("s1", 1_600, 0, None)?,
        ],
    )?;
    assert_eq!(
        found,
        [PersistedFailure {
            runner: name("pytest")?,
            edited: vec![nodes[0]],
            until: vec![nodes[1]],
        }]
    );
    Ok(())
}

/// Each case differs from the worked example above by one step, and none is a persisted failure:
/// the second failure named other tests (Y), so the first did not stay; nothing changed between
/// the two failures, so it outlasted no change; the change before the pass was to `a.py` again,
/// no new file; a git restore ran while it failed, so what was changed is not known; the run
/// before the pass was not read for its tests; and the check never passed.
#[test]
fn a_failure_that_did_not_stay_the_same_is_no_fact() -> TestResult {
    let mut store = Store::new()?;
    store.index(&["a.py", "b.py"])?;
    let other_tests = [
        edit("s1", 1_100, "a.py", &[(1, 1)])?,
        pytest("s1", 1_200, 1, Some("X"))?,
        edit("s1", 1_300, "a.py", &[(2, 2)])?,
        pytest("s1", 1_400, 1, Some("Y"))?,
        edit("s1", 1_500, "b.py", &[(1, 1)])?,
        pytest("s1", 1_600, 0, None)?,
    ];
    let no_change_between = [
        edit("s2", 1_100, "a.py", &[(1, 1)])?,
        pytest("s2", 1_200, 1, Some("X"))?,
        pytest("s2", 1_400, 1, Some("X"))?,
        edit("s2", 1_500, "b.py", &[(1, 1)])?,
        pytest("s2", 1_600, 0, None)?,
    ];
    let no_new_file = [
        edit("s3", 1_100, "a.py", &[(1, 1)])?,
        pytest("s3", 1_200, 1, Some("X"))?,
        edit("s3", 1_300, "a.py", &[(2, 2)])?,
        pytest("s3", 1_400, 1, Some("X"))?,
        edit("s3", 1_500, "a.py", &[(3, 3)])?,
        pytest("s3", 1_600, 0, None)?,
    ];
    let restored = [
        edit("s4", 1_100, "a.py", &[(1, 1)])?,
        pytest("s4", 1_200, 1, Some("X"))?,
        edit("s4", 1_300, "a.py", &[(2, 2)])?,
        pytest("s4", 1_400, 1, Some("X"))?,
        shell("s4", 1_450, "git restore a.py", Some(0))?,
        edit("s4", 1_500, "b.py", &[(1, 1)])?,
        pytest("s4", 1_600, 0, None)?,
    ];
    let unread = [
        edit("s5", 1_100, "a.py", &[(1, 1)])?,
        pytest("s5", 1_200, 1, Some("X"))?,
        edit("s5", 1_300, "a.py", &[(2, 2)])?,
        pytest("s5", 1_400, 1, None)?,
        edit("s5", 1_500, "b.py", &[(1, 1)])?,
        pytest("s5", 1_600, 0, None)?,
    ];
    let never_passed = [
        edit("s6", 1_100, "a.py", &[(1, 1)])?,
        pytest("s6", 1_200, 1, Some("X"))?,
        edit("s6", 1_300, "a.py", &[(2, 2)])?,
        pytest("s6", 1_400, 1, Some("X"))?,
        edit("s6", 1_500, "b.py", &[(1, 1)])?,
    ];
    for (session, steps) in [
        ("s1", &other_tests[..]),
        ("s2", &no_change_between[..]),
        ("s3", &no_new_file[..]),
        ("s4", &restored[..]),
        ("s5", &unread[..]),
        ("s6", &never_passed[..]),
    ] {
        assert_eq!(persisted(&mut store, session, steps)?, [], "{session}");
    }
    Ok(())
}

/// What a file holds at a task's end: `text`'s hash, read at `read_at_ms`.
fn holding(text: &str, read_at_ms: i64) -> FileNow {
    FileNow {
        content: FileContent::Hashed(ContentHash::of(text.as_bytes())),
        read_at_ms,
    }
}

/// `a.py` held `one` and `b.py` `two` before the task, which edits both and then `c.py`, which
/// held `three`. At its end `a.py` holds `one` again, read after its edit: it was changed back.
/// `b.py` holds something else. `c.py` holds `three` again, but as read before the task's edit of
/// it, which says nothing. Only `a.py` was changed back, and the task's record keeps it among the
/// files it put back, with the persisted failure of its pytest runs.
#[test]
fn a_file_back_as_it_was_was_changed_back() -> TestResult {
    let mut store = Store::new()?;
    let nodes: [NodeId; 3] = store
        .index_text_at(&[("a.py", "one"), ("b.py", "two"), ("c.py", "three")], 10)?
        .try_into()
        .map_err(|_| "three nodes")?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &[
            edit("s1", 1_100, "a.py", &[(1, 1)])?,
            pytest("s1", 1_150, 1, Some("X"))?,
            edit("s1", 1_200, "b.py", &[(1, 1)])?,
            pytest("s1", 1_250, 1, Some("X"))?,
            edit("s1", 1_300, "c.py", &[(1, 1)])?,
            pytest("s1", 1_350, 0, None)?,
        ],
    )?;
    let now = BTreeMap::from([
        (RepoPath::new("a.py")?, holding("one", 2_000)),
        (RepoPath::new("b.py")?, holding("changed", 2_000)),
        (RepoPath::new("c.py")?, holding("three", 1_250)),
    ]);
    assert_eq!(changed_back(&state, &now), [nodes[0]]);
    let record = episode_of(
        store.writer.queries(),
        &state,
        &now,
        (TaskOutcome::Success, 3_000),
    )?;
    assert_eq!(record.pitfalls.undone, [nodes[0]]);
    assert_eq!(
        record.pitfalls.persisted,
        [PersistedFailure {
            runner: name("pytest")?,
            edited: vec![nodes[0], nodes[1]],
            until: vec![nodes[2]],
        }]
    );
    Ok(())
}

/// A failure of `cargo test` ends only at a pass of the same test run. A worked control: `cargo
/// test` fails naming X, `a.py` changes, it fails naming X again, `b.py` changes, and `cargo test`
/// passes: a persisted failure of `a.py` until `b.py`. The same steps with `cargo check` (a build,
/// not a test run) passing at the end, and with `pytest` passing after a streak of `pytest -k
/// other`'s failures (another selection), give nothing: neither ends the failing run.
#[test]
fn only_a_pass_of_the_same_test_run_ends_a_failure() -> TestResult {
    let mut store = Store::new()?;
    let nodes: [NodeId; 2] = store
        .index(&["a.py", "b.py"])?
        .try_into()
        .map_err(|_| "two nodes")?;
    let steps = |session: &str, failing: &str, passing: &str| -> TestResult<Vec<Event>> {
        Ok(vec![
            edit(session, 1_100, "a.py", &[(1, 1)])?,
            check((session, 1_200), failing, 1, Some("X"))?,
            edit(session, 1_300, "a.py", &[(2, 2)])?,
            check((session, 1_400), failing, 1, Some("X"))?,
            edit(session, 1_500, "b.py", &[(1, 1)])?,
            check((session, 1_600), passing, 0, None)?,
        ])
    };
    assert_eq!(
        persisted(&mut store, "s1", &steps("s1", "cargo test", "cargo test")?)?,
        [PersistedFailure {
            runner: name("cargo")?,
            edited: vec![nodes[0]],
            until: vec![nodes[1]],
        }]
    );
    let build = steps("s2", "cargo test", "cargo check")?;
    assert_eq!(persisted(&mut store, "s2", &build)?, []);
    let other = steps("s3", "pytest -k other", "pytest")?;
    assert_eq!(persisted(&mut store, "s3", &other)?, []);
    Ok(())
}

/// A git restore before the failure began that put back a file among those changed by the last
/// failing run leaves what was changed unknown. The task edits `a.py` and `c.py`, restores `c.py`,
/// then pytest fails naming X, `a.py` changes, it fails naming X again, `b.py` changes and it
/// passes: nothing. The control restores `d.py`, which the task never changed: a persisted failure
/// of `a.py` and `c.py` until `b.py`.
#[test]
fn a_restore_of_a_changed_file_before_the_failure_gives_nothing() -> TestResult {
    let mut store = Store::new()?;
    let nodes: [NodeId; 3] = store
        .index(&["a.py", "b.py", "c.py"])?
        .try_into()
        .map_err(|_| "three nodes")?;
    let steps = |session: &str, restored: &str| -> TestResult<Vec<Event>> {
        Ok(vec![
            edit(session, 1_100, "a.py", &[(1, 1)])?,
            edit(session, 1_110, "c.py", &[(1, 1)])?,
            shell(session, 1_120, &format!("git restore {restored}"), Some(0))?,
            pytest(session, 1_200, 1, Some("X"))?,
            edit(session, 1_300, "a.py", &[(2, 2)])?,
            pytest(session, 1_400, 1, Some("X"))?,
            edit(session, 1_500, "b.py", &[(1, 1)])?,
            pytest(session, 1_600, 0, None)?,
        ])
    };
    assert_eq!(persisted(&mut store, "s1", &steps("s1", "c.py")?)?, []);
    assert_eq!(
        persisted(&mut store, "s2", &steps("s2", "d.py")?)?,
        [PersistedFailure {
            runner: name("pytest")?,
            edited: vec![nodes[0], nodes[2]],
            until: vec![nodes[1]],
        }]
    );
    Ok(())
}

/// Steps rebuilt from rows keep no restore, so a task whose state was rebuilt gives no persisted
/// failure. The first recorder sees the start, the prompt and the edit of `a.py`; a second, as
/// after the daemon started again, sees the rest (pytest fails naming X, `a.py` changes, it fails
/// naming X again, `b.py` changes, it passes) and rebuilds the first steps from the store.
#[test]
fn a_rebuilt_state_gives_no_persisted_failure() -> TestResult {
    let mut store = Store::new()?;
    store.index(&["a.py", "b.py"])?;
    let mut first = Recorder::new(false, 0.0, Commands::default());
    store.record(
        &mut first,
        &[
            start("s1", 1_000, StartSource::Startup)?,
            prompt("s1", 1_001, "fix it")?,
            edit("s1", 1_100, "a.py", &[(1, 1)])?,
        ],
    )?;
    let mut second = Recorder::new(false, 0.0, Commands::default());
    let ended = store.record(
        &mut second,
        &[
            pytest("s1", 1_200, 1, Some("X"))?,
            edit("s1", 1_300, "a.py", &[(2, 2)])?,
            pytest("s1", 1_400, 1, Some("X"))?,
            edit("s1", 1_500, "b.py", &[(1, 1)])?,
            pytest("s1", 1_600, 0, None)?,
            event(HookKind::Stop, "s1", 2_000)?,
        ],
    )?;
    let [state] = ended.as_slice() else {
        return Err("one task ended".into());
    };
    assert!(state.rebuilt_steps() > 0);
    assert_eq!(persisted_failures(state), []);
    Ok(())
}

/// A restore counts as putting a file back only after the task's last change of it. The task edits
/// `a.py`, restores it, edits it again and ends with it changed; it edits `c.py` and restores it.
/// Its record keeps only `c.py` among the files it put back.
#[test]
fn a_file_changed_again_after_its_restore_was_not_put_back() -> TestResult {
    let mut store = Store::new()?;
    let nodes: [NodeId; 2] = store
        .index(&["a.py", "c.py"])?
        .try_into()
        .map_err(|_| "two nodes")?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let state = answered_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        &[
            edit("s1", 1_100, "a.py", &[(1, 1)])?,
            shell("s1", 1_200, "git restore a.py", Some(0))?,
            edit("s1", 1_300, "a.py", &[(2, 2)])?,
            edit("s1", 1_400, "c.py", &[(1, 1)])?,
            shell("s1", 1_500, "git restore c.py", Some(0))?,
        ],
    )?;
    let record = episode_of(
        store.writer.queries(),
        &state,
        &BTreeMap::new(),
        (TaskOutcome::Unknown, 3_000),
    )?;
    assert_eq!(record.pitfalls.undone, [nodes[1]]);
    Ok(())
}
