//! Edits only the file watcher saw: how long each is held, when a hook's edit claims it, and which
//! session it is recorded for, if any. The rows expected are worked out by hand from the events.

mod common;

use common::{Store, TestResult, end, event, failed, name, prompt, start, target, task_of, tool};
use cortex_core::{
    ContentHash, EventKind, EventMeta, EventSource, FileFlags, HookKind, Identifier, Language,
    RepoPath, StartSource, ToolKind, Ulid,
};
use cortex_learn::{Recorder, WatcherEdit, WatcherEdits};
use cortex_store::{EventRecord, FileIndex, FileRecord, NodeId};

/// A repository path.
fn path(text: &str) -> TestResult<RepoPath> {
    Ok(RepoPath::new(text)?)
}

/// Records the edit of `file` the watcher saw at `seen_ms`, in a transaction of its own.
fn watcher_edit(
    store: &mut Store,
    recorder: &mut Recorder,
    (file, seen_ms): (&str, i64),
) -> TestResult<WatcherEdit> {
    let mut transaction = store.writer.transaction()?;
    let edit = recorder.record_watcher_edit(&mut transaction, &path(file)?, seen_ms)?;
    transaction.commit()?;
    Ok(edit)
}

/// The row an edit of `file` the watcher saw at `seen_ms` is stored as, for `session` in `task`.
fn watched(
    file: &str,
    seen_ms: i64,
    (session, task): (Option<&str>, Option<Ulid>),
    node_id: Option<NodeId>,
) -> TestResult<EventRecord> {
    Ok(EventRecord {
        task_id: task,
        session_id: session.map(name).transpose()?,
        ts_ms: seen_ms,
        kind: EventKind::Edit,
        node_id,
        path: Some(path(file)?),
        range: None,
        meta: EventMeta {
            source: Some(EventSource::Watcher),
            ..EventMeta::default()
        },
    })
}

/// The rows stored that the watcher reported, in order.
fn watcher_rows(store: &Store) -> TestResult<Vec<EventRecord>> {
    Ok(store
        .events()?
        .into_iter()
        .filter(|row| row.meta.source == Some(EventSource::Watcher))
        .collect())
}

/// A session of `name`.
fn session(text: &str) -> TestResult<WatcherEdit> {
    Ok(WatcherEdit::Session(name(text)?))
}

/// Each edit is held 2 s from when the watcher first saw it. `a.py` seen at 1,000 ms and again at
/// 1,500 keeps 1,000, so it is due at 3,000 with `b.py`, not at 2,999; the two come out together,
/// in path order. `c.py`, seen at 1,200, is due at 3,200. Nothing is held after.
#[test]
fn edits_are_held_two_seconds_from_when_they_were_first_seen() -> TestResult {
    let mut edits = WatcherEdits::default();
    edits.hold([path("b.py")?, path("a.py")?], 1_000);
    edits.hold([path("a.py")?], 1_500);
    edits.hold([path("c.py")?], 1_200);
    assert_eq!(edits.next_due_ms(), Some(3_000));
    assert_eq!(edits.take_due(2_999), []);
    assert_eq!(
        edits.take_due(3_000),
        [(path("a.py")?, 1_000), (path("b.py")?, 1_000)]
    );
    assert_eq!(edits.next_due_ms(), Some(3_200));
    assert_eq!(edits.take_due(9_000), [(path("c.py")?, 1_200)]);
    assert_eq!(edits.next_due_ms(), None);
    Ok(())
}

/// An edit nobody reported goes to the live session whose latest event is the most recent and at
/// most 30 s older than the edit, in that session's task.
///
/// `s1` starts at 10,000 and prompts at 11,000 (task T1); `s2` starts at 12,000. The edit of
/// `a.py` at 20,000 is `s2`'s, whose latest event is the more recent, and `s2` has no task. `s1`
/// reads at 21,000; the edit of `b.py` at 22,000 is then `s1`'s, in T1. The edit of `c.py` at
/// 51,500 needs an event since 21,500: `s1`'s latest is 21,000 and `s2`'s 12,000, so it is
/// nobody's. `s1` ends at 52,000; the edit of `d.py` at 52,500 is nobody's, an ended session being
/// no candidate.
#[test]
fn an_edit_goes_to_the_session_most_recently_active() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let asked = prompt("s1", 11_000, "fix it")?;
    let t1 = task_of(&asked)?;
    store.record(
        &mut recorder,
        &[
            start("s1", 10_000, StartSource::Startup)?,
            asked,
            start("s2", 12_000, StartSource::Startup)?,
        ],
    )?;
    let read = tool(
        HookKind::PostTool,
        ("s1", 21_000),
        (ToolKind::Read, target("b.py", &[])?, None),
    )?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("a.py", 20_000))?,
        session("s2")?
    );
    store.record(&mut recorder, &[read])?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("b.py", 22_000))?,
        session("s1")?
    );
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("c.py", 51_500))?,
        WatcherEdit::Nobody
    );
    store.record(&mut recorder, &[end("s1", 52_000, "exit")?])?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("d.py", 52_500))?,
        WatcherEdit::Nobody
    );
    assert_eq!(
        watcher_rows(&store)?,
        [
            watched("a.py", 20_000, (Some("s2"), None), None)?,
            watched("b.py", 22_000, (Some("s1"), Some(t1)), None)?,
            watched("c.py", 51_500, (None, None), None)?,
            watched("d.py", 52_500, (None, None), None)?,
        ]
    );
    Ok(())
}

/// A hook's edit of the same file at most 2 s before or after the watcher saw it is the same edit:
/// `s1` edits `d.py` at 30,000, so the watcher's edits of it at 28,500 and 31,500 are reported
/// and leave no row; at 32,500 it is another edit, `s1`'s. `e.py`, which no hook edited, is
/// recorded at 31,000.
#[test]
fn a_hook_s_edit_claims_the_watcher_s() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let asked = prompt("s1", 11_000, "fix it")?;
    let t1 = task_of(&asked)?;
    let edit = tool(
        HookKind::PostTool,
        ("s1", 30_000),
        (ToolKind::Edit, target("d.py", &[(1, 2)])?, None),
    )?;
    store.record(
        &mut recorder,
        &[start("s1", 10_000, StartSource::Startup)?, asked, edit],
    )?;
    for seen_ms in [28_500, 31_500] {
        assert_eq!(
            watcher_edit(&mut store, &mut recorder, ("d.py", seen_ms))?,
            WatcherEdit::Reported
        );
    }
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("e.py", 31_000))?,
        session("s1")?
    );
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("d.py", 32_500))?,
        session("s1")?
    );
    assert_eq!(
        watcher_rows(&store)?,
        [
            watched("e.py", 31_000, (Some("s1"), Some(t1)), None)?,
            watched("d.py", 32_500, (Some("s1"), Some(t1)), None)?,
        ]
    );
    Ok(())
}

/// An edit that failed changed nothing, so it claims no edit the watcher saw: `s1`'s Edit of
/// `d.py` at 30,000 fails, and the watcher's edit of `d.py` at 31,000 (a command's, say), within
/// 2 s of it, is recorded as `s1`'s, in T1.
#[test]
fn a_failed_edit_claims_nothing() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let asked = prompt("s1", 11_000, "fix it")?;
    let t1 = task_of(&asked)?;
    let failed_edit = tool(
        HookKind::PostTool,
        ("s1", 30_000),
        (ToolKind::Edit, target("d.py", &[])?, failed()),
    )?;
    store.record(
        &mut recorder,
        &[
            start("s1", 10_000, StartSource::Startup)?,
            asked,
            failed_edit,
        ],
    )?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("d.py", 31_000))?,
        session("s1")?
    );
    assert_eq!(
        watcher_rows(&store)?,
        [watched("d.py", 31_000, (Some("s1"), Some(t1)), None)?]
    );
    Ok(())
}

/// A subagent's work counts for the session that started it, which waits on it: `s1` prompts at
/// 11,000 (task T1) and its subagent `s1/a1` starts at 12,000 and reads at 30,000 and 44,000. The
/// edit at 45,000 is `s1`'s, in T1, though `s1`'s own latest event is 34 s older; and once the
/// subagent stops at 46,000, the edit at 47,000 is `s1`'s too.
#[test]
fn a_subagent_s_work_counts_for_its_starter() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let asked = prompt("s1", 11_000, "fix it")?;
    let t1 = task_of(&asked)?;
    let child = |hook, ts_ms| -> TestResult<_> {
        Ok(cortex_core::Event {
            parent_session_id: Some(name("s1")?),
            ..event(hook, "s1/a1", ts_ms)?
        })
    };
    let read = |ts_ms| -> TestResult<_> {
        Ok(cortex_core::Event {
            tool: Some(ToolKind::Read),
            tool_target: Some(target("a.py", &[])?),
            ..child(HookKind::PostTool, ts_ms)?
        })
    };
    store.record(
        &mut recorder,
        &[
            start("s1", 10_000, StartSource::Startup)?,
            asked,
            child(HookKind::SubagentStart, 12_000)?,
            read(30_000)?,
            read(44_000)?,
        ],
    )?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("b.py", 45_000))?,
        session("s1")?
    );
    store.record(&mut recorder, &[child(HookKind::SubagentStop, 46_000)?])?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("c.py", 47_000))?,
        session("s1")?
    );
    assert_eq!(
        watcher_rows(&store)?,
        [
            watched("b.py", 45_000, (Some("s1"), Some(t1)), None)?,
            watched("c.py", 47_000, (Some("s1"), Some(t1)), None)?,
        ]
    );
    Ok(())
}

/// Whose an edit is does not depend on the order events came in, only on their times and the
/// edit's. `s1` ends at 20,000, and a Stop of `s1` stamped 20,000 arrives after the end: the edit at
/// 21,000 is still nobody's, `s1` having ended before it. `s2` reads at 31,000 and ends at 32,000,
/// and its end is recorded before the edit seen at 31,500 is weighed: the edit is `s2`'s, made
/// before its end. `s3` starts at 39,600, more than 2 s after the edit seen at 37,500: the edit is
/// nobody's, `s2` having ended before it.
#[test]
fn the_owner_does_not_depend_on_arrival_order() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    store.record(
        &mut recorder,
        &[
            start("s1", 10_000, StartSource::Startup)?,
            end("s1", 20_000, "exit")?,
            event(HookKind::Stop, "s1", 20_000)?,
        ],
    )?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("a.py", 21_000))?,
        WatcherEdit::Nobody
    );
    let read = tool(
        HookKind::PostTool,
        ("s2", 31_000),
        (ToolKind::Read, target("b.py", &[])?, None),
    )?;
    store.record(
        &mut recorder,
        &[
            start("s2", 30_000, StartSource::Startup)?,
            read,
            end("s2", 32_000, "exit")?,
        ],
    )?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("b.py", 31_500))?,
        session("s2")?
    );
    store.record(&mut recorder, &[start("s3", 39_600, StartSource::Startup)?])?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("c.py", 37_500))?,
        WatcherEdit::Nobody
    );
    Ok(())
}

/// What the recorder saw outlives a transaction that was rolled back: after
/// [`Recorder::forget_stored`], `s1`'s hook-reported edit of `a.py` at 11,000 still claims the
/// watcher's at 12,000, and the edit of `b.py` at 12,000 is still `s1`'s. So does every report
/// of a file within 2 s, not only the latest: `c.py` edited by hooks at 13,000 and 23,000 claims
/// the watcher's edit seen at 13,500.
#[test]
fn what_was_seen_outlives_a_rolled_back_batch() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let edit = |ts_ms, file| {
        tool(
            HookKind::PostTool,
            ("s1", ts_ms),
            (ToolKind::Edit, target(file, &[(1, 1)])?, None),
        )
    };
    store.record(
        &mut recorder,
        &[
            start("s1", 10_000, StartSource::Startup)?,
            edit(11_000, "a.py")?,
            edit(13_000, "c.py")?,
            edit(23_000, "c.py")?,
        ],
    )?;
    recorder.forget_stored();
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("a.py", 12_000))?,
        WatcherEdit::Reported
    );
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("b.py", 12_000))?,
        session("s1")?
    );
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("c.py", 13_500))?,
        WatcherEdit::Reported
    );
    Ok(())
}

/// Of two sessions whose latest events are as recent, the edit goes to the one whose id sorts
/// first, whatever the order they came in; and an edit of an indexed file names the file's node.
#[test]
fn ties_go_to_the_first_session_by_id() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false);
    let mut transaction = store.writer.transaction()?;
    transaction.replace_file(&FileIndex {
        file: FileRecord {
            path: path("a.py")?,
            language: Some(Language::Python),
            size_bytes: 6,
            line_count: Some(1),
            content_hash: Some(ContentHash::of(b"x = 1\n")),
            mtime_ns: 1,
            flags: FileFlags::empty(),
            indexed_at_ms: 1,
        },
        symbols: Vec::new(),
        refs: Vec::new(),
        passes_on: Vec::new(),
    })?;
    transaction.commit()?;
    let node = store.writer.queries().file_node_at(&path("a.py")?)?;
    assert!(node.is_some());
    store.record(
        &mut recorder,
        &[
            start("s2", 10_000, StartSource::Startup)?,
            start("s1", 10_000, StartSource::Startup)?,
        ],
    )?;
    let first: Identifier = name("s1")?;
    assert_eq!(
        watcher_edit(&mut store, &mut recorder, ("a.py", 11_000))?,
        WatcherEdit::Session(first)
    );
    assert_eq!(
        watcher_rows(&store)?,
        [watched("a.py", 11_000, (Some("s1"), None), node)?]
    );
    Ok(())
}
