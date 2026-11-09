//! The use of the lines naming the file earlier tasks read next, counted as each task ends, and
//! kept apart from the labels that judge work orders.

mod common;

use common::{Store, TestResult, event, prompt, read, start, task_of};
use cortex_core::{Config, ContentHash, HookKind, Identifier, ReasonKind, StartSource};
use cortex_learn::{Commands, Learner, Recorder};
use cortex_store::{InjectedFile, InjectionRecord};

/// Worked by hand. A task is told after a read of `a.py` that earlier tasks read `b.py` next, and
/// after another that they read `c.py` next; it reads `a.py` and `b.py`, never `c.py`. At its end
/// the two lines are counted: 2 named, 1 used (`b.py`). Neither gets a label, so the delivered,
/// labelled work orders the self-check looks at stay at 0.
#[test]
fn a_next_file_line_counts_as_used_when_the_task_reads_the_file() -> TestResult {
    let mut store = Store::new()?;
    let [_, b, c] = store.index(&["a.py", "b.py", "c.py"])?[..] else {
        return Err("three nodes".into());
    };
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    let asked = prompt("s1", 101, "fix it")?;
    store.record(
        &mut recorder,
        &[start("s1", 100, StartSource::Startup)?, asked.clone()],
    )?;
    let task = task_of(&asked)?;
    let session = Identifier::new("s1")?;
    let line = |node, at_ms| InjectionRecord {
        task_id: Some(task),
        session_id: session.clone(),
        hook: HookKind::PreTool,
        at_ms,
        files: vec![InjectedFile {
            node,
            spans: Vec::new(),
            reason: Some(ReasonKind::ReadNext),
            shown: Vec::new(),
            content: None,
        }],
        tokens: 20,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the line")),
        features: None,
    };
    let mut transaction = store.writer.transaction()?;
    transaction.record_injection(&line(b, 110))?;
    transaction.record_injection(&line(c, 120))?;
    transaction.commit()?;
    let ended = store.record(
        &mut recorder,
        &[
            read("s1", 130, "a.py", &[])?,
            read("s1", 140, "b.py", &[])?,
            event(HookKind::Stop, "s1", 200)?,
        ],
    )?;
    let state = ended.first().ok_or("no task ended")?;
    let mut transaction = store.writer.transaction()?;
    Learner::new()
        .learn(&mut transaction, state, &Config::default())?
        .ok_or("nothing learnt")?;
    transaction.commit()?;
    let queries = store.writer.queries();
    assert_eq!(queries.next_file_use()?, (2, 1));
    assert!(
        queries
            .injections_of_task(task)?
            .iter()
            .all(|row| row.label.is_none())
    );
    assert_eq!(queries.delivered_labelled()?, 0);
    Ok(())
}
