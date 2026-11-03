//! What a task's end teaches, as one transaction writes it: the judgement and its words, the
//! interest, the links, the record of the task and the clusters; and the look the next task's end
//! in the same session takes at the task before it.

mod common;

use common::{Store, TestResult, event, prompt, shell, start, succeeded, target, tool};
use cortex_core::{
    Config, EdgeType, Event, HookKind, InterestTier, StartSource, TaskOutcome, ToolKind, Ulid,
};
use cortex_learn::{Commands, Learned, Learner, MAX_SESSIONS_KEPT, Recorder, cluster_tasks};
use cortex_store::{EdgeRecord, NodeId};

/// An edit of line 1 of `path` by `session` at `ts_ms`, as Claude Code reports one that applied.
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

/// The events of one task of `session` from `at_ms`: its prompt, then `steps`, then the end of its
/// answer 400 ms after it started.
fn task(session: &str, at_ms: i64, steps: Vec<Event>) -> TestResult<Vec<Event>> {
    let mut events = vec![prompt(session, at_ms, "change it")?];
    events.extend(steps);
    events.push(event(HookKind::Stop, session, at_ms + 400)?);
    Ok(events)
}

/// The recorder and the learner the tests feed, as the daemon holds them.
struct Learning {
    /// What turns events into rows.
    recorder: Recorder,
    /// What learns from the tasks that end.
    learner: Learner,
}

impl Learning {
    /// A recorder that tells the runners Cortex knows, and a learner that has seen nothing.
    fn new() -> Self {
        Self {
            recorder: Recorder::new(false, Commands::default()),
            learner: Learner::new(),
        }
    }

    /// Records `events` in `store` and learns from each task they end, each in a transaction of its
    /// own with the default settings, kept once committed; each task ended and what its end found,
    /// in order.
    fn feed(&mut self, store: &mut Store, events: &[Event]) -> TestResult<Vec<(Ulid, Learned)>> {
        let mut found = Vec::new();
        for state in store.record(&mut self.recorder, events)? {
            let task_id = state.task_id();
            let mut transaction = store.writer.transaction()?;
            let learned = self
                .learner
                .learn(&mut transaction, &state, &Config::default())?
                .ok_or("a task with no ended row")?;
            transaction.commit()?;
            self.learner.keep(store.writer.queries(), state)?;
            found.push((task_id, learned));
        }
        Ok(found)
    }

    /// Records `events`, which must end exactly one task, and learns from it as [`Learning::feed`]
    /// does.
    fn one(&mut self, store: &mut Store, events: &[Event]) -> TestResult<(Ulid, Learned)> {
        let mut found = self.feed(store, events)?.into_iter();
        let first = found.next().ok_or("no task ended")?;
        assert!(found.next().is_none(), "one task only");
        Ok(first)
    }
}

/// The judgement stored for `task_id` and the words that say why.
fn judged(store: &Store, task_id: Ulid) -> TestResult<(Option<TaskOutcome>, Option<String>)> {
    let row = store.writer.queries().task(task_id)?.ok_or("no task")?;
    Ok((row.outcome, row.outcome_evidence))
}

/// The outcome the record of `task_id` holds.
fn recorded(store: &Store, task_id: Ulid) -> TestResult<TaskOutcome> {
    let record = store
        .writer
        .queries()
        .episode_of_task(task_id)?
        .ok_or("no record")?;
    Ok(record.outcome)
}

/// One link as compared here: from, to, weight, count, and the task it last changed at.
type Link = (NodeId, NodeId, f64, u32, i64);

/// The links stored among `nodes`.
fn links(store: &Store, nodes: &[NodeId]) -> TestResult<Vec<Link>> {
    Ok(store
        .writer
        .queries()
        .edges_among(nodes, EdgeType::CoAccess)?
        .into_iter()
        .map(|row| {
            let EdgeRecord {
                dst,
                weight,
                count,
                last_seq,
                ..
            } = row.edge;
            (row.src, dst, weight, count, last_seq)
        })
        .collect())
}

/// `a.py` and `b.py`, stored as the index read them at the start: `a` and `b`.
fn two_files(store: &mut Store) -> TestResult<(NodeId, NodeId)> {
    let nodes = store.index_text_at(&[("a.py", "a\n"), ("b.py", "b\n")], 1)?;
    Ok((
        nodes.first().copied().ok_or("no file")?,
        nodes.get(1).copied().ok_or("no file")?,
    ))
}

/// A task that changes two files and then passes its tests is a success, and its end leaves every
/// mark, worked out by hand from the settings' defaults. The words: `cargo`'s tests exited 0 after
/// the last edit. Each file earns 1.0 for its edit and 0.5 for the passing run after it, 1.5,
/// which is interesting and short of a landmark's 3.0. The two edited files are linked once, by the
/// learning rate 0.1 times both roles 1.0 times a success's full gain: 0.1. The record says what
/// changed and that it succeeded. The one cluster holds the two files, `calc/ops.py` first as the
/// smaller node, and the words of their paths in the order met: `calc`, `ops`, `py`, `util`.
#[test]
fn a_task_that_passed_its_tests_leaves_every_mark() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let nodes = store.index_text_at(
        &[("calc/ops.py", "x = 1\n"), ("calc/util.py", "y = 1\n")],
        1,
    )?;
    let (ops, util) = (
        nodes.first().copied().ok_or("no file")?,
        nodes.get(1).copied().ok_or("no file")?,
    );
    let mut events = vec![start("s1", 1_000, StartSource::Startup)?];
    events.extend(task(
        "s1",
        1_001,
        vec![
            edit("s1", 1_002, "calc/ops.py")?,
            edit("s1", 1_003, "calc/util.py")?,
            shell("s1", 1_004, "cargo test", Some(0))?,
        ],
    )?);
    let (task_id, found) = learning.one(&mut store, &events)?;
    assert_eq!(
        found,
        Learned {
            outcome: TaskOutcome::Success,
            reverted_before: None,
        }
    );
    assert_eq!(
        judged(&store, task_id)?,
        (
            Some(TaskOutcome::Success),
            Some("tests by cargo exited 0 after the last edit".to_owned())
        )
    );
    let interest: Vec<(NodeId, f64, u64, InterestTier)> = store
        .writer
        .queries()
        .interest_of(&[ops, util])?
        .into_iter()
        .map(|row| (row.node, row.score, row.last_seq, row.tier))
        .collect();
    assert_eq!(
        interest,
        [
            (ops, 1.5, 1, InterestTier::Interesting),
            (util, 1.5, 1, InterestTier::Interesting),
        ]
    );
    assert_eq!(links(&store, &[ops, util])?, [(ops, util, 0.1, 1, 1)]);
    let record = store
        .writer
        .queries()
        .episode_of_task(task_id)?
        .ok_or("no record")?;
    assert_eq!(record.outcome, TaskOutcome::Success);
    let changed: Vec<NodeId> = record.change.iter().map(|file| file.node).collect();
    assert_eq!(changed, [ops, util]);
    let mut transaction = store.writer.transaction()?;
    cluster_tasks(&mut transaction, 2_000)?;
    transaction.commit()?;
    let clusters = store.writer.queries().archetypes()?;
    let words: Vec<Vec<&str>> = clusters
        .iter()
        .map(|cluster| {
            cluster
                .seed_terms
                .iter()
                .map(|term| term.as_str())
                .collect()
        })
        .collect();
    assert_eq!(words, [vec!["calc", "ops", "py", "util"]]);
    let shapes: Vec<(Vec<NodeId>, u32, i64)> = clusters
        .into_iter()
        .map(|cluster| (cluster.files, cluster.episode_count, cluster.updated_at_ms))
        .collect();
    assert_eq!(shapes, [(vec![ops, util], 1, 2_000)]);
    Ok(())
}

/// Tasks that fail or are left teach the links nothing: one changes `a.py` and `b.py` and its tests
/// then fail; another changes both and is left when its session's next prompt comes before the
/// end of its answer. Neither links the files, and each is judged as it ended.
#[test]
fn failed_and_left_tasks_link_nothing() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let (a, b) = two_files(&mut store)?;
    let mut events = vec![start("s1", 1_000, StartSource::Startup)?];
    events.extend(task(
        "s1",
        1_001,
        vec![
            edit("s1", 1_002, "a.py")?,
            edit("s1", 1_003, "b.py")?,
            shell("s1", 1_004, "cargo test", Some(1))?,
        ],
    )?);
    events.extend([
        prompt("s1", 2_000, "change it")?,
        edit("s1", 2_002, "a.py")?,
        edit("s1", 2_003, "b.py")?,
    ]);
    events.extend(task("s1", 3_000, Vec::new())?);
    let found = learning.feed(&mut store, &events)?;
    let outcomes: Vec<TaskOutcome> = found.iter().map(|(_, learned)| learned.outcome).collect();
    assert_eq!(
        outcomes,
        [
            TaskOutcome::Failure,
            TaskOutcome::Abandoned,
            TaskOutcome::Unknown
        ]
    );
    let failed = found
        .first()
        .map(|(task_id, _)| *task_id)
        .ok_or("no task")?;
    assert_eq!(
        judged(&store, failed)?,
        (
            Some(TaskOutcome::Failure),
            Some("tests by cargo exited 1 after the last edit".to_owned())
        )
    );
    assert_eq!(links(&store, &[a, b])?, []);
    Ok(())
}

/// A revert the index shows only after a task's end is found at the end of the session's next
/// task. The first task changes `a.py` and `b.py` to `a2` and `b2`, which the index reads before
/// the task ends: not back, and nothing checked, so it is unknown, which links the files by 0.1
/// times both roles times three tenths: 0.03. Then the files are put back as they were and the
/// index reads them so, and the session's next task, which changes nothing, ends: every file the
/// first changed is back, so the first is judged reverted, with words saying so by the end of the
/// next task. Its link is weakened at task 2: what decay leaves of 0.03 over one task, 0.02985,
/// less 0.05, is held at nothing, its second observation. Its record says it was put back.
#[test]
fn a_revert_the_index_shows_late_is_found_at_the_next_tasks_end() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let (a, b) = two_files(&mut store)?;
    let opening = [
        start("s1", 1_000, StartSource::Startup)?,
        prompt("s1", 1_001, "change it")?,
        edit("s1", 1_002, "a.py")?,
        edit("s1", 1_003, "b.py")?,
    ];
    assert!(learning.feed(&mut store, &opening)?.is_empty());
    store.index_text_at(&[("a.py", "a2\n"), ("b.py", "b2\n")], 1_300)?;
    let (first_id, found) = learning.one(&mut store, &[event(HookKind::Stop, "s1", 1_401)?])?;
    assert_eq!(
        found,
        Learned {
            outcome: TaskOutcome::Unknown,
            reverted_before: None,
        }
    );
    assert_eq!(
        judged(&store, first_id)?,
        (Some(TaskOutcome::Unknown), None)
    );
    assert_eq!(links(&store, &[a, b])?, [(a, b, 0.03, 1, 1)]);

    store.index_text_at(&[("a.py", "a\n"), ("b.py", "b\n")], 1_500)?;
    let (_, found) = learning.one(&mut store, &task("s1", 2_000, Vec::new())?)?;
    assert_eq!(
        found,
        Learned {
            outcome: TaskOutcome::Unknown,
            reverted_before: Some(first_id),
        }
    );
    assert_eq!(
        judged(&store, first_id)?,
        (
            Some(TaskOutcome::Reverted),
            Some(
                "every edited file is back as it was before the task, by the end of the next task"
                    .to_owned()
            )
        )
    );
    assert_eq!(links(&store, &[a, b])?, [(a, b, 0.0, 2, 2)]);
    assert_eq!(recorded(&store, first_id)?, TaskOutcome::Reverted);
    Ok(())
}

/// A success the next task puts back with git keeps its gain and loses a reverted task's share.
/// The first task changes `a.py` and `b.py` and passes its tests: linked by 0.1 at task 1. The
/// session's next task restores both with git and ends: the first is judged reverted, by the end
/// of the next task, and its link weakened at task 2 to what decay leaves of 0.1 over one task,
/// 0.1 x 0.995 = 0.0995, less 0.05: 0.0495, its second observation.
#[test]
fn a_success_put_back_later_keeps_its_gain_less_a_reverts_share() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let (a, b) = two_files(&mut store)?;
    let mut events = vec![start("s1", 1_000, StartSource::Startup)?];
    events.extend(task(
        "s1",
        1_001,
        vec![
            edit("s1", 1_002, "a.py")?,
            edit("s1", 1_003, "b.py")?,
            shell("s1", 1_004, "cargo test", Some(0))?,
        ],
    )?);
    let (first_id, found) = learning.one(&mut store, &events)?;
    assert_eq!(found.outcome, TaskOutcome::Success);
    assert_eq!(links(&store, &[a, b])?, [(a, b, 0.1, 1, 1)]);
    let restore = task(
        "s1",
        2_000,
        vec![shell("s1", 2_002, "git restore a.py b.py", Some(0))?],
    )?;
    let (_, found) = learning.one(&mut store, &restore)?;
    assert_eq!(
        found,
        Learned {
            outcome: TaskOutcome::Unknown,
            reverted_before: Some(first_id),
        }
    );
    assert_eq!(
        judged(&store, first_id)?,
        (
            Some(TaskOutcome::Reverted),
            Some(
                "every edited file was put back or restored by git, by the end of the next task"
                    .to_owned()
            )
        )
    );
    assert_eq!(links(&store, &[a, b])?, [(a, b, 0.0495, 2, 2)]);
    assert_eq!(recorded(&store, first_id)?, TaskOutcome::Reverted);
    Ok(())
}

/// A task whose work stood is left alone at the next end: the first task changes `a.py` and
/// `b.py` and passes its tests, linked by 0.1; the index reads them changed; the session's next
/// task ends having done nothing, and the first keeps its judgement, its link and its record.
#[test]
fn a_task_whose_work_stood_is_left_alone() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let (a, b) = two_files(&mut store)?;
    let mut events = vec![start("s1", 1_000, StartSource::Startup)?];
    events.extend(task(
        "s1",
        1_001,
        vec![
            edit("s1", 1_002, "a.py")?,
            edit("s1", 1_003, "b.py")?,
            shell("s1", 1_004, "cargo test", Some(0))?,
        ],
    )?);
    let (first_id, _) = learning.one(&mut store, &events)?;
    store.index_text_at(&[("a.py", "a2\n"), ("b.py", "b2\n")], 1_500)?;
    let (_, found) = learning.one(&mut store, &task("s1", 2_000, Vec::new())?)?;
    assert_eq!(found.reverted_before, None);
    assert_eq!(
        judged(&store, first_id)?,
        (
            Some(TaskOutcome::Success),
            Some("tests by cargo exited 0 after the last edit".to_owned())
        )
    );
    assert_eq!(links(&store, &[a, b])?, [(a, b, 0.1, 1, 1)]);
    assert_eq!(recorded(&store, first_id)?, TaskOutcome::Success);
    Ok(())
}

/// A file the store holds no row for is not taken as gone: a task creates `new.py`, which held
/// nothing before it, and ends before the index has seen the file. Nothing says when the file
/// went or whether it is there now, so nothing shows it back as it was, and the task is unknown
/// rather than reverted.
#[test]
fn a_new_file_the_index_has_not_seen_is_not_taken_as_gone() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let mut events = vec![start("s1", 1_000, StartSource::Startup)?];
    events.extend(task("s1", 1_001, vec![edit("s1", 1_002, "new.py")?])?);
    let (task_id, found) = learning.one(&mut store, &events)?;
    assert_eq!(found.outcome, TaskOutcome::Unknown);
    assert_eq!(judged(&store, task_id)?, (Some(TaskOutcome::Unknown), None));
    Ok(())
}

/// A task judged reverted at its own end is not weakened again at the next. A task of session
/// `s0` changes `a.py` and `b.py` and passes its tests, linking them by 0.1. A task of `s1` changes
/// both and restores them with git: reverted at once, which weakens the link at task 2 to 0.0495
/// (0.1 x 0.995 - 0.05), its second observation. The next task of `s1` ends having done nothing,
/// and finds its predecessor already judged: the link stays.
#[test]
fn a_task_judged_reverted_at_its_end_is_not_weakened_twice() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let (a, b) = two_files(&mut store)?;
    let mut events = vec![
        start("s0", 1_000, StartSource::Startup)?,
        start("s1", 1_000, StartSource::Startup)?,
    ];
    events.extend(task(
        "s0",
        1_001,
        vec![
            edit("s0", 1_002, "a.py")?,
            edit("s0", 1_003, "b.py")?,
            shell("s0", 1_004, "cargo test", Some(0))?,
        ],
    )?);
    events.extend(task(
        "s1",
        2_001,
        vec![
            edit("s1", 2_002, "a.py")?,
            edit("s1", 2_003, "b.py")?,
            shell("s1", 2_004, "git restore a.py b.py", Some(0))?,
        ],
    )?);
    let found = learning.feed(&mut store, &events)?;
    let restored = found
        .get(1)
        .map(|(task_id, _)| *task_id)
        .ok_or("no second task")?;
    assert_eq!(
        judged(&store, restored)?,
        (
            Some(TaskOutcome::Reverted),
            Some("every edited file was put back or restored by git".to_owned())
        )
    );
    assert_eq!(links(&store, &[a, b])?, [(a, b, 0.0495, 2, 2)]);
    let (_, found) = learning.one(&mut store, &task("s1", 3_000, Vec::new())?)?;
    assert_eq!(found.reverted_before, None);
    assert_eq!(links(&store, &[a, b])?, [(a, b, 0.0495, 2, 2)]);
    Ok(())
}

/// Each session's last task is kept for the next end's look, for at most 32 sessions, the one whose
/// last task ended longest ago let go first. Session `s00` starts a task that changes `a.py` and
/// runs long; 32 other sessions each run one that changes it too, start to end, in turn; then
/// `s00`'s ends, last of all though it started first. The index reads `a.py` as it was. `s00`'s
/// next task finds its first put back, as it was kept; `s01`'s finds nothing to look at, as its
/// task ended first and was let go for `s00`'s.
#[test]
fn only_so_many_sessions_keep_their_last_task() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    store.index_text_at(&[("a.py", "a\n")], 1)?;
    let long = [
        start("s00", 1_000, StartSource::Startup)?,
        prompt("s00", 1_001, "change it")?,
        edit("s00", 1_002, "a.py")?,
    ];
    assert!(learning.feed(&mut store, &long)?.is_empty());
    let others: Vec<String> = (1..=MAX_SESSIONS_KEPT)
        .map(|at| format!("s{at:02}"))
        .collect();
    assert_eq!(others.len(), 32);
    for (at, session) in others.iter().enumerate() {
        let at_ms = 2_000 + i64::try_from(at)? * 1_000;
        let mut events = vec![start(session, at_ms, StartSource::Startup)?];
        events.extend(task(
            session,
            at_ms + 1,
            vec![edit(session, at_ms + 2, "a.py")?],
        )?);
        learning.one(&mut store, &events)?;
    }
    let (long_id, _) = learning.one(&mut store, &[event(HookKind::Stop, "s00", 60_000)?])?;
    store.index_text_at(&[("a.py", "a\n")], 100_000)?;
    let (_, found) = learning.one(&mut store, &task("s00", 200_000, Vec::new())?)?;
    assert_eq!(
        found.reverted_before,
        Some(long_id),
        "s00's last task was kept"
    );
    let (_, found) = learning.one(&mut store, &task("s01", 201_000, Vec::new())?)?;
    assert_eq!(found.reverted_before, None, "s01's last task was let go");
    Ok(())
}
