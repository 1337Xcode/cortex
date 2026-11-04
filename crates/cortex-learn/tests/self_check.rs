//! The self-check as tasks end: the labels of the work orders that reached the agent, counted at
//! each task's end, raise the gate's bar once the latest hundred are used too rarely.

mod common;

use common::{Given, Store, TestResult, given_task, read, start};
use cortex_core::{Config, ContentHash, HookKind, Identifier, StartSource};
use cortex_learn::{Commands, Learner, Recorder, TaskState};
use cortex_store::{InjectionRecord, SelfCheckRecord};

/// Learns from `state` in `store` as the daemon does.
fn learn(store: &mut Store, state: &TaskState) -> TestResult {
    let mut transaction = store.writer.transaction()?;
    Learner::new()
        .learn(&mut transaction, state, &Config::default())?
        .ok_or("nothing learnt")?;
    transaction.commit()?;
    Ok(())
}

/// Worked by hand: 99 work orders that reached the agent were labelled unused before. A task whose
/// work order the gate kept back labels it, but adds nothing the self-check counts, which does not
/// run. Then a task whose delivered work order gave `c.py`, which it never touched (it read
/// `e.py`), is the hundredth: 0 of 100 used, below 0.6, so the bar is raised one step, at 100.
#[test]
fn the_hundredth_unused_work_order_raises_the_bar() -> TestResult {
    let mut store = Store::new()?;
    let [c, _] = store.index(&["c.py", "e.py"])?[..] else {
        return Err("two nodes".into());
    };
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    store.record(&mut recorder, &[start("s0", 100, StartSource::Startup)?])?;
    let mut transaction = store.writer.transaction()?;
    for at in 0..99 {
        let id = transaction.record_injection(&InjectionRecord {
            task_id: None,
            session_id: Identifier::new("s0")?,
            hook: HookKind::Prompt,
            at_ms: 200 + at,
            files: Vec::new(),
            tokens: 10,
            confidence: None,
            is_gated_out: false,
            is_holdout: false,
            text_hash: Some(ContentHash::of(b"an earlier work order")),
            features: None,
        })?;
        transaction.label_injection(id, 0, false)?;
    }
    transaction.commit()?;
    let given = [(c, &[(1, 2)][..])];
    let kept = given_task(
        (&mut store, &mut recorder),
        ("s1", 1_000),
        (&given, Given::KeptBack),
        &[read("s1", 1_010, "e.py", &[])?],
    )?;
    learn(&mut store, &kept)?;
    assert_eq!(
        store.writer.queries().self_check()?,
        SelfCheckRecord::default()
    );
    let shown = given_task(
        (&mut store, &mut recorder),
        ("s2", 2_000),
        (&given, Given::Shown),
        &[read("s2", 2_010, "e.py", &[])?],
    )?;
    learn(&mut store, &shown)?;
    assert_eq!(
        store.writer.queries().self_check()?,
        SelfCheckRecord {
            raise_steps: 1,
            raised_at: 100,
            recent_labelled: 100,
            recent_used: 0,
        }
    );
    Ok(())
}
