//! What upkeep stops keeping, deleted in bounded batches, worked out by hand: events past their
//! retention, episodes past their cap (unknown outcomes first, then the oldest), positions that
//! can no longer hold, and facts retired long ago with their anchors and where they were given.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{
    Agent, EventKind, EventMeta, FactKind, FactSource, FactStatus, Identifier, PositionMode,
    TaskOutcome, Ulid,
};
use cortex_store::{
    EpisodeRecord, EventRecord, FactRecord, Pitfalls, PositionRecord, SessionRecord, TaskRecord,
    WriteTx,
};

/// Stores session `s1` and, for each of `outcomes`, a task started at 1,000 plus its position and
/// its episode made at 2,000 plus its position; returns the tasks.
fn tasks_with_episodes(
    transaction: &mut WriteTx<'_>,
    outcomes: &[TaskOutcome],
) -> TestResult<Vec<Ulid>> {
    transaction.record_session(&SessionRecord {
        session_id: Identifier::new("s1")?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 500,
    })?;
    let mut tasks = Vec::new();
    for (at, outcome) in (0_i64..).zip(outcomes) {
        let task_id = Ulid::from_seed(u64::try_from(at)?, b"task");
        transaction.start_task(&TaskRecord {
            task_id,
            session_id: Identifier::new("s1")?,
            started_at_ms: 1_000 + at,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: None,
        })?;
        transaction.replace_episode(&EpisodeRecord {
            task_id,
            localize: Vec::new(),
            change: Vec::new(),
            verify: Vec::new(),
            pitfalls: Pitfalls::default(),
            outcome: *outcome,
            created_at_ms: 2_000 + at,
        })?;
        tasks.push(task_id);
    }
    Ok(tasks)
}

/// Worked by hand. Events at 1,000, 2,000 and 3,000: deleting those before 2,500 one at a time
/// takes the one at 1,000, then 2,000, then none; 3,000 is kept. Episodes of four tasks, made at
/// 2,000 to 2,003, outcomes success, unknown, failure, unknown: kept to two, the unknown ones go
/// (2,001 and 2,003), leaving the first and third; kept to two again, none. Positions last at 100
/// and 900: those before 500 go, one.
#[test]
fn old_events_episodes_past_the_cap_and_old_positions_are_deleted() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let a = transaction
        .replace_file(&index("a.py", Vec::new(), Vec::new())?)?
        .file_node;
    for ts_ms in [1_000, 2_000, 3_000] {
        transaction.append_event(&EventRecord {
            task_id: None,
            session_id: None,
            ts_ms,
            kind: EventKind::Read,
            node_id: None,
            path: None,
            range: None,
            meta: EventMeta::default(),
        })?;
    }
    assert_eq!(transaction.delete_events_before(2_500, 1)?, 1);
    assert_eq!(transaction.delete_events_before(2_500, 1)?, 1);
    assert_eq!(transaction.delete_events_before(2_500, 1)?, 0);
    let outcomes = [
        TaskOutcome::Success,
        TaskOutcome::Unknown,
        TaskOutcome::Failure,
        TaskOutcome::Unknown,
    ];
    let tasks = tasks_with_episodes(&mut transaction, &outcomes)?;
    assert_eq!(transaction.cap_episodes(2, 10)?, 2);
    assert_eq!(transaction.cap_episodes(2, 10)?, 0);
    let mut kept: Vec<Ulid> = transaction
        .queries()
        .episodes(10)?
        .into_iter()
        .map(|episode| episode.task_id)
        .collect();
    kept.sort_unstable();
    let mut expected = vec![tasks[0], tasks[2]];
    expected.sort_unstable();
    assert_eq!(kept, expected);
    for (session, last_ms) in [("s1", 100), ("s2", 900)] {
        transaction.keep_position(&PositionRecord {
            session_id: Identifier::new(session)?,
            node: a,
            mode: PositionMode::Read,
            since_ms: last_ms,
            last_ms,
        })?;
    }
    assert_eq!(transaction.delete_positions_before(500)?, 1);
    assert_eq!(transaction.queries().positions_since(0)?.len(), 1);
    transaction.commit()?;
    Ok(())
}

/// A new store gives back the pages deletions free: after 2,000 events are deleted some pages are
/// free, and optimizing gives back every one its statistics do not take. It would give none back
/// from a store that does not vacuum incrementally.
#[test]
fn a_new_store_gives_back_the_pages_deletions_free() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    for ts_ms in 0..2_000 {
        transaction.append_event(&EventRecord {
            task_id: None,
            session_id: Some(Identifier::new("a-session-with-a-long-name")?),
            ts_ms,
            kind: EventKind::Read,
            node_id: None,
            path: None,
            range: None,
            meta: EventMeta::default(),
        })?;
    }
    transaction.commit()?;
    let mut transaction = writer.transaction()?;
    assert_eq!(transaction.delete_events_before(2_000, 5_000)?, 2_000);
    transaction.commit()?;
    let free = writer.queries().free_pages()?;
    assert!(free > 0, "{free}");
    let mut transaction = writer.transaction()?;
    // Refreshing the planner's statistics may take a free page or two for them first.
    let given = transaction.optimize(u32::MAX)?;
    assert!(given > 0 && given <= free, "{given} of {free}");
    transaction.commit()?;
    assert_eq!(writer.queries().free_pages()?, 0);
    Ok(())
}

/// Worked by hand. Three facts on `a.py`, each given in a task: fact 1 retired at 100, fact 2
/// retired at 900, fact 3 active. Deleting the facts retired before 500 removes fact 1 alone, with
/// its anchor and the record of where it was given; facts 2 and 3 keep theirs.
#[test]
fn facts_retired_long_ago_are_deleted_with_their_anchors() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let a = transaction
        .replace_file(&index("a.py", Vec::new(), Vec::new())?)?
        .file_node;
    let anchors = transaction.queries().anchor_hashes(&[a])?;
    let task = Ulid::from_seed(7, b"task");
    let mut ids = Vec::new();
    for text in ["One rule.", "Two rules.", "Three rules."] {
        ids.push(transaction.add_fact(&FactRecord {
            text: text.to_owned(),
            kind: FactKind::Convention,
            source: FactSource::User,
            status: FactStatus::Active,
            created_at_ms: 1,
            task: None,
            anchors: anchors.clone(),
        })?);
    }
    transaction.give_facts(Some(task), &ids)?;
    transaction.set_fact_status(ids[0], FactStatus::Retired, 100)?;
    transaction.set_fact_status(ids[1], FactStatus::Retired, 900)?;
    assert_eq!(transaction.delete_retired_facts_before(500, 10)?, 1);
    assert_eq!(transaction.queries().fact(ids[0])?, None);
    assert!(transaction.queries().fact(ids[1])?.is_some());
    assert_eq!(
        transaction.queries().facts_given_in(task)?,
        [ids[1], ids[2]]
    );
    assert_eq!(transaction.delete_retired_facts_before(500, 10)?, 0);
    transaction.commit()?;
    Ok(())
}
