//! What a task's end teaches, as one transaction writes it: the judgement and its words, the
//! interest, the links, the record of the task and the clusters; and the look the next task's end
//! in the same session takes at the task before it.

mod common;

use common::{Store, TestResult, end, event, prompt, shell, start, succeeded, target, tool};
use cortex_core::{
    CommandKind, Config, EdgeType, Event, HookKind, Identifier, InterestTier, StartSource,
    TaskOutcome, ToolKind, Ulid,
};
use cortex_learn::{Commands, Learned, Learner, MAX_SESSIONS_KEPT, Recorder, cluster_tasks};
use cortex_store::{EdgeRecord, HandoffEnd, HandoffRecord, LastFailure, NodeId};

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
            recorder: Recorder::new(false, 0.0, Commands::default()),
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
            self.learner
                .keep(store.writer.queries(), state, learned.linked.clone())?;
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

/// What one end found, as compared here: what became of the task and the task before it found put
/// back.
fn found_as(learned: &Learned) -> (TaskOutcome, Option<Ulid>) {
    (learned.outcome, learned.reverted_before)
}

/// Whether `link` is `expected`, its weight within a rounding error: a weight worked out by hand
/// through decay is not always the float the sums give.
fn is_link(link: Option<&Link>, expected: Link) -> bool {
    link.is_some_and(|(src, dst, weight, count, last_seq)| {
        (*src, *dst, *count, *last_seq) == (expected.0, expected.1, expected.3, expected.4)
            && (weight - expected.2).abs() < 1e-12
    })
}

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
    assert_eq!(found_as(&found), (TaskOutcome::Success, None));
    assert_eq!(found.linked, [((ops, util), 0.1)].into_iter().collect());
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
/// next task. At task 2 its link loses what the first gave it, which decayed as the link did, and
/// a reverted task's 0.05: 0.02985 - 0.02985 - 0.05, held at nothing, its second observation. Its
/// record says it was put back.
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
    assert_eq!(found_as(&found), (TaskOutcome::Unknown, None));
    assert_eq!(
        judged(&store, first_id)?,
        (Some(TaskOutcome::Unknown), None)
    );
    assert_eq!(links(&store, &[a, b])?, [(a, b, 0.03, 1, 1)]);

    store.index_text_at(&[("a.py", "a\n"), ("b.py", "b\n")], 1_500)?;
    let (_, found) = learning.one(&mut store, &task("s1", 2_000, Vec::new())?)?;
    assert_eq!(found_as(&found), (TaskOutcome::Unknown, Some(first_id)));
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

/// A success the next task puts back with git loses what it gave the links, at both levels, and
/// what other work gave them stays. `a.py` holds `f` and `b.py` holds `g`, both on lines 1 and 2,
/// so an edit of line 1 touches the file and its symbol. A task of `s0` edits both files and
/// passes its tests: `a`-`b` and `f`-`g` gain 0.1 each at task 1. A task of `s1` does the same:
/// each link is 0.1 x 0.995 + 0.1 = 0.1995 at task 2, its second observation. The next task of
/// `s1` restores both files with git and ends: the task before it is judged reverted, by the end
/// of the next task. At task 3 each link loses the 0.1 that task gave it, decayed over one task to
/// 0.0995, from 0.1995 x 0.995 = 0.1985025: 0.0990025, what `s0`'s 0.1 alone decays to over two
/// tasks, 0.1 x 0.995^2. The files' link loses a reverted task's 0.05 as well: 0.0490025. Both are
/// their third observation.
#[test]
fn a_success_put_back_later_loses_what_it_gave_the_links() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let a_nodes = store.index_symbols("a.py", &[("f", 1, 2)])?;
    let (a, f) = (
        a_nodes.first().copied().ok_or("no file")?,
        a_nodes.get(1).copied().ok_or("no symbol")?,
    );
    let b_nodes = store.index_symbols("b.py", &[("g", 1, 2)])?;
    let (b, g) = (
        b_nodes.first().copied().ok_or("no file")?,
        b_nodes.get(1).copied().ok_or("no symbol")?,
    );
    let mut events = vec![
        start("s0", 1_000, StartSource::Startup)?,
        start("s1", 1_000, StartSource::Startup)?,
    ];
    for (session, at_ms) in [("s0", 1_001), ("s1", 2_001)] {
        events.extend(task(
            session,
            at_ms,
            vec![
                edit(session, at_ms + 1, "a.py")?,
                edit(session, at_ms + 2, "b.py")?,
                shell(session, at_ms + 3, "cargo test", Some(0))?,
            ],
        )?);
    }
    let found = learning.feed(&mut store, &events)?;
    let first_id = found
        .get(1)
        .map(|(task_id, _)| *task_id)
        .ok_or("no task of s1")?;
    let both: Vec<TaskOutcome> = found.iter().map(|(_, learned)| learned.outcome).collect();
    assert_eq!(both, [TaskOutcome::Success, TaskOutcome::Success]);
    let held = links(&store, &[a, b, f, g])?;
    assert!(is_link(held.first(), (a, b, 0.1995, 2, 2)), "{held:?}");
    assert!(is_link(held.get(1), (f, g, 0.1995, 2, 2)), "{held:?}");
    let restore = task(
        "s1",
        3_000,
        vec![shell("s1", 3_002, "git restore a.py b.py", Some(0))?],
    )?;
    let (_, found) = learning.one(&mut store, &restore)?;
    assert_eq!(found_as(&found), (TaskOutcome::Unknown, Some(first_id)));
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
    let held = links(&store, &[a, b, f, g])?;
    assert_eq!(held.len(), 2, "{held:?}");
    assert!(is_link(held.first(), (a, b, 0.049_002_5, 3, 3)), "{held:?}");
    assert!(is_link(held.get(1), (f, g, 0.099_002_5, 3, 3)), "{held:?}");
    assert_eq!(recorded(&store, first_id)?, TaskOutcome::Reverted);
    Ok(())
}

/// What a late revert takes back is what its task's end wrote, after the ceiling of 5.0 took its
/// part: `a.py` and `b.py` are linked by 4.95 as of task 0, before any task here. A task of `s1`
/// edits both and passes its tests, which earns 0.1 on top of 4.95 x 0.995 = 4.92525: 5.02525,
/// held at 5.0, so 0.07475 was written, the link's second observation. The next task of `s1`
/// restores both with git: at task 2 the link is 5.0 x 0.995 = 4.975, less what was written
/// decayed alike, 0.07475 x 0.995 = 0.07437625, less a reverted task's 0.05: 4.85062375, what
/// 4.95 alone leaves after two tasks, 4.90062375, less that share. Taking back the 0.1 earned
/// would have left 4.8255.
#[test]
fn a_late_revert_takes_back_what_the_ceiling_left() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let (a, b) = two_files(&mut store)?;
    let mut transaction = store.writer.transaction()?;
    let planted = EdgeRecord {
        dst: b,
        weight: 4.95,
        count: 1,
        last_seq: 0,
        is_directed: false,
    };
    transaction.set_edges(EdgeType::CoAccess, &[(a, planted)])?;
    transaction.commit()?;
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
    assert_eq!(links(&store, &[a, b])?, [(a, b, 5.0, 2, 1)]);
    let written = found
        .linked
        .get(&(a, b))
        .copied()
        .ok_or("nothing written")?;
    assert!((written - 0.074_75).abs() < 1e-12, "{written}");
    let restore = task(
        "s1",
        2_000,
        vec![shell("s1", 2_002, "git restore a.py b.py", Some(0))?],
    )?;
    let (_, found) = learning.one(&mut store, &restore)?;
    assert_eq!(found_as(&found), (TaskOutcome::Unknown, Some(first_id)));
    let held = links(&store, &[a, b])?;
    assert!(
        is_link(held.first(), (a, b, 4.850_623_75, 3, 2)),
        "{held:?}"
    );
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

/// Tasks that did not succeed hand off, worked out by hand. `s1` edits `a.py`, reads `b.py`, sees
/// `cargo test` exit 1, and its session ends before its answer did: it failed, and leaves, as left,
/// `a.py` edited, `b.py` read and the failed run. `s2` edits `b.py` and its answer ends at 2,401
/// with no check: unknown, it leaves, as answered, `b.py` edited. `s3` edits `b.py` and its turn
/// stops on a usage limit: abandoned, it leaves, as stopped, `b.py` edited and no failure; its
/// next task edits only `a.py` and passes, which closes nothing; the one after edits `b.py`, passes
/// `cargo test` and is a success, which closes the handoff
/// `s3` left. `s1`'s and `s2`'s stay open, the newest first.
#[test]
fn a_task_that_did_not_succeed_leaves_a_handoff() -> TestResult {
    let mut store = Store::new()?;
    let mut learning = Learning::new();
    let (a, b) = two_files(&mut store)?;
    let read_b = |session, ts_ms| {
        tool(
            HookKind::PostTool,
            (session, ts_ms),
            (ToolKind::Read, target("b.py", &[])?, succeeded(None, None)),
        )
    };
    let mut events = vec![
        start("s1", 1_000, StartSource::Startup)?,
        prompt("s1", 1_001, "change it")?,
        edit("s1", 1_002, "a.py")?,
        read_b("s1", 1_003)?,
        shell("s1", 1_004, "cargo test", Some(1))?,
        end("s1", 1_500, "other")?,
        start("s2", 2_000, StartSource::Startup)?,
    ];
    events.extend(task("s2", 2_001, vec![edit("s2", 2_002, "b.py")?])?);
    events.extend([
        start("s3", 3_000, StartSource::Startup)?,
        prompt("s3", 3_001, "change it")?,
        edit("s3", 3_002, "b.py")?,
        Event {
            end_reason: Some(Identifier::new("rate_limit")?),
            ..event(HookKind::Stop, "s3", 3_003)?
        },
    ]);
    let found = learning.feed(&mut store, &events)?;
    let outcomes: Vec<TaskOutcome> = found.iter().map(|(_, learned)| learned.outcome).collect();
    assert_eq!(
        outcomes,
        [
            TaskOutcome::Failure,
            TaskOutcome::Unknown,
            TaskOutcome::Abandoned
        ]
    );
    let tasks: Vec<Ulid> = found.iter().map(|(task_id, _)| *task_id).collect();
    let open = |store: &Store| -> TestResult<Vec<HandoffRecord>> {
        Ok(store
            .writer
            .queries()
            .open_handoffs_since(0)?
            .into_iter()
            .map(|row| row.record)
            .collect())
    };
    let left_by_s1 = HandoffRecord {
        from_session: Identifier::new("s1")?,
        task_id: tasks[0],
        end: HandoffEnd::Left,
        edited: vec![a],
        read: vec![b],
        open_warnings: Vec::new(),
        last_failure: Some(LastFailure {
            runner: Identifier::new("cargo")?,
            kind: CommandKind::Test,
            exit_code: 1,
        }),
        created_at_ms: 1_500,
    };
    let left_by_s2 = HandoffRecord {
        from_session: Identifier::new("s2")?,
        task_id: tasks[1],
        end: HandoffEnd::Answered,
        edited: vec![b],
        read: Vec::new(),
        open_warnings: Vec::new(),
        last_failure: None,
        created_at_ms: 2_401,
    };
    let left_by_s3 = HandoffRecord {
        from_session: Identifier::new("s3")?,
        task_id: tasks[2],
        end: HandoffEnd::Stopped,
        edited: vec![b],
        read: Vec::new(),
        open_warnings: Vec::new(),
        last_failure: None,
        created_at_ms: 3_003,
    };
    assert_eq!(
        open(&store)?,
        [left_by_s3.clone(), left_by_s2.clone(), left_by_s1.clone()]
    );
    // A success on another file settles none of it: `s3`'s handoff names `b.py`, this task edits
    // only `a.py`.
    let elsewhere = task(
        "s3",
        3_500,
        vec![
            edit("s3", 3_501, "a.py")?,
            shell("s3", 3_502, "cargo test", Some(0))?,
        ],
    )?;
    let (_, learned) = learning.one(&mut store, &elsewhere)?;
    assert_eq!(learned.outcome, TaskOutcome::Success);
    assert_eq!(
        open(&store)?,
        [left_by_s3, left_by_s2.clone(), left_by_s1.clone()]
    );
    let finished = task(
        "s3",
        4_000,
        vec![
            edit("s3", 4_001, "b.py")?,
            shell("s3", 4_002, "cargo test", Some(0))?,
        ],
    )?;
    let (_, learned) = learning.one(&mut store, &finished)?;
    assert_eq!(learned.outcome, TaskOutcome::Success);
    assert_eq!(open(&store)?, [left_by_s2, left_by_s1]);
    Ok(())
}
