//! Clusters of tasks: written whole, read back in order, replaced together, and the records the
//! clustering reads them from. Expected values are written by hand.

mod common;

use common::{Scratch, TestResult, index};
use cortex_core::{Agent, Identifier, TaskOutcome, Ulid};
use cortex_store::{
    ArchetypeRecord, EpisodeRecord, NodeId, Pitfalls, SessionRecord, TaskRecord, Writer,
};

/// A name.
fn name(text: &str) -> TestResult<Identifier> {
    Ok(Identifier::new(text)?)
}

/// A cluster of `episode_count` tasks over `files`, found by the words `terms`.
fn archetype(terms: &[&str], files: &[NodeId], episode_count: u32) -> TestResult<ArchetypeRecord> {
    Ok(ArchetypeRecord {
        seed_terms: terms
            .iter()
            .map(|term| name(term))
            .collect::<TestResult<Vec<Identifier>>>()?,
        files: files.to_vec(),
        episode_count,
        updated_at_ms: 5_000,
    })
}

/// A store with one file stored, for the clusters' nodes to point at.
fn stored(scratch: &Scratch) -> TestResult<(Writer, NodeId)> {
    let mut writer = scratch.writer(1_000)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&index("a.py", Vec::new(), Vec::new())?)?;
    let node = transaction
        .queries()
        .file_node_at(&common::path("a.py")?)?
        .ok_or("a.py has no node")?;
    transaction.commit()?;
    Ok((writer, node))
}

/// Clusters are written whole and read back in the order they were written, and writing again
/// replaces every one of them: two clusters are written, then one, and that one is all there is.
#[test]
fn clusters_are_written_whole_and_replaced_together() -> TestResult {
    let scratch = Scratch::new()?;
    let (mut writer, node) = stored(&scratch)?;
    let first = archetype(&["shop", "orders"], &[node], 3)?;
    let second = archetype(&["calc"], &[], 1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_archetypes(&[first.clone(), second.clone()])?;
    transaction.commit()?;
    assert_eq!(writer.queries().archetypes()?, [first, second]);
    let only = archetype(&["parser"], &[node], 7)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_archetypes(std::slice::from_ref(&only))?;
    transaction.commit()?;
    assert_eq!(writer.queries().archetypes()?, [only]);
    let mut transaction = writer.transaction()?;
    transaction.replace_archetypes(&[])?;
    transaction.commit()?;
    assert_eq!(writer.queries().archetypes()?, []);
    Ok(())
}

/// The clustering reads the newest records, oldest first, and no more than it asked for: three
/// tasks are recorded, and reading two gives the two newest, which are the two whose ids sort last.
#[test]
fn the_clustering_reads_the_newest_records_oldest_first() -> TestResult {
    let scratch = Scratch::new()?;
    let (mut writer, _) = stored(&scratch)?;
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: name("s1")?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 500,
    })?;
    let mut ids = Vec::new();
    for at in 0..3 {
        let task_id = Ulid::from_seed(1_000 + at, b"task");
        transaction.start_task(&TaskRecord {
            task_id,
            session_id: name("s1")?,
            started_at_ms: 1_000 + i64::try_from(at)?,
            prompt_hash: None,
            prompt_excerpt: None,
        })?;
        transaction.replace_episode(&EpisodeRecord {
            task_id,
            localize: Vec::new(),
            change: Vec::new(),
            verify: Vec::new(),
            pitfalls: Pitfalls::default(),
            outcome: TaskOutcome::Success,
            created_at_ms: 2_000 + i64::try_from(at)?,
        })?;
        ids.push(task_id);
    }
    transaction.commit()?;
    ids.sort_unstable();
    let read: Vec<Ulid> = writer
        .queries()
        .episodes(2)?
        .into_iter()
        .map(|record| record.task_id)
        .collect();
    assert_eq!(read, ids[1..], "the newest two, oldest first");
    assert_eq!(writer.queries().episodes(10)?.len(), 3);
    Ok(())
}
