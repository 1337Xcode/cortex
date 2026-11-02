//! One task's record: written, read back, replaced, refused when its parts are too large, and
//! gone with its task. Expected values are written by hand.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{Agent, CommandKind, Identifier, TaskOutcome, Ulid};
use cortex_store::{
    ChangedFile, ChangedSymbol, EpisodeRecord, MAX_EPISODE_BYTES, NodeId, Pitfalls, RanCheck,
    ReadAgain, ReadFile, SessionRecord, StoreError, TaskRecord, Writer,
};

/// A name.
fn name(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
}

/// A Claude Code session, with nothing optional known.
fn session(id: &str) -> TestResult<SessionRecord> {
    Ok(SessionRecord {
        session_id: name(id)?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 500,
    })
}

/// A task of `session_id`, started at `started_at_ms`, with no prompt kept.
fn task_record(task_id: Ulid, session_id: &str, started_at_ms: i64) -> TestResult<TaskRecord> {
    Ok(TaskRecord {
        task_id,
        session_id: name(session_id)?,
        started_at_ms,
        prompt_hash: None,
        prompt_excerpt: None,
    })
}

/// The task whose record these tests write, with a file stored for its nodes to point at.
fn task(scratch: &Scratch) -> TestResult<(Writer, Ulid, NodeId)> {
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&index("a.py", Vec::new(), Vec::new())?)?;
    let node = transaction
        .queries()
        .file_node_at(&common::path("a.py")?)?
        .ok_or("a.py has no node")?;
    let task_id = Ulid::from_seed(1_000, b"task");
    transaction.record_session(&session("s1")?)?;
    transaction.start_task(&task_record(task_id, "s1", 1_000)?)?;
    transaction.commit()?;
    Ok((writer, task_id, node))
}

/// A record of one task, with `reads` files read, one file changed and one check run.
fn record(task_id: Ulid, node: NodeId, reads: u32) -> TestResult<EpisodeRecord> {
    Ok(EpisodeRecord {
        task_id,
        localize: vec![ReadFile { node, reads }],
        change: vec![ChangedFile {
            node,
            lines: 12,
            symbols: vec![ChangedSymbol { node, header: true }],
        }],
        verify: vec![RanCheck {
            runner: name("cargo")?,
            kind: CommandKind::Test,
            code: Some(0),
        }],
        pitfalls: Pitfalls {
            fixed: vec![name("pytest")?],
            undone: vec![node],
            read_again: vec![ReadAgain {
                node,
                from_line: 10,
                to_line: 20,
                times: 4,
            }],
        },
        outcome: TaskOutcome::Success,
        created_at_ms: 2_000,
    })
}

/// A record is written as it was given, read back the same, and replaced in place: the task's
/// second record, which read its file three times instead of once, is the one that is kept.
#[test]
fn a_record_is_written_read_back_and_replaced() -> TestResult {
    let scratch = Scratch::new()?;
    let (mut writer, task_id, node) = task(&scratch)?;
    let first = record(task_id, node, 1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_episode(&first)?;
    transaction.commit()?;
    assert_eq!(writer.queries().episode_of_task(task_id)?, Some(first));
    let again = record(task_id, node, 3)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_episode(&again)?;
    transaction.commit()?;
    assert_eq!(writer.queries().episode_of_task(task_id)?, Some(again));
    assert_eq!(
        writer
            .queries()
            .episode_of_task(Ulid::from_seed(2_000, b"other"))?,
        None
    );
    Ok(())
}

/// A record whose parts hold more than the bound as JSON together is refused rather than written,
/// however small each part is on its own. Counted by hand: 50 files read hold 1,091 bytes and 18
/// files changed hold 18 x 58 with 17 commas and two brackets, 1,063, which with the one check's
/// 43 and the pitfalls' 97 is 2,294 of the 2,048 the parts may hold together, while neither part
/// comes near the bound by itself. The task is left without a record.
#[test]
fn a_record_past_its_bound_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    let (mut writer, task_id, node) = task(&scratch)?;
    let mut too_large = record(task_id, node, 1)?;
    too_large.localize = (0..50).map(|reads| ReadFile { node, reads }).collect();
    too_large.change = (0..18)
        .map(|_| ChangedFile {
            node,
            lines: 12,
            symbols: vec![ChangedSymbol { node, header: true }],
        })
        .collect();
    for part in too_large.parts()? {
        assert!(
            part.len() <= MAX_EPISODE_BYTES,
            "no part should reach the bound on its own"
        );
    }
    assert!(
        too_large.size_bytes()? > MAX_EPISODE_BYTES,
        "the record should be too large"
    );
    let mut transaction = writer.transaction()?;
    let refused = transaction.replace_episode(&too_large);
    assert!(
        matches!(refused, Err(StoreError::TooLarge { column, .. }) if column == "episodes"),
        "a record past its bound should be refused, got {refused:?}"
    );
    transaction.commit()?;
    assert_eq!(writer.queries().episode_of_task(task_id)?, None);
    Ok(())
}

/// A record just inside the bound is written. Counted by hand: a file read is
/// `{"node":1,"reads":0}`, 20 bytes while the count of reads is one digit and 21 while it is two,
/// so 80 of them with counts 0 to 79 hold 10 x 20 + 70 x 21 = 1,670 bytes, with 79 commas and two
/// brackets 1,751; the one changed file holds 60, the one check 43 and the pitfalls 97, which is
/// 1,951 of the 2,048 the parts may hold together.
#[test]
fn a_record_inside_its_bound_is_written() -> TestResult {
    let scratch = Scratch::new()?;
    let (mut writer, task_id, node) = task(&scratch)?;
    let mut inside = record(task_id, node, 1)?;
    inside.localize = (0..80).map(|reads| ReadFile { node, reads }).collect();
    assert!(
        inside.size_bytes()? <= MAX_EPISODE_BYTES,
        "the record should fit"
    );
    let mut transaction = writer.transaction()?;
    transaction.replace_episode(&inside)?;
    transaction.commit()?;
    assert_eq!(
        writer
            .queries()
            .episode_of_task(task_id)?
            .map(|held| held.localize.len()),
        Some(80)
    );
    Ok(())
}
