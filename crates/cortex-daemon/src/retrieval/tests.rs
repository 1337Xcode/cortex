//! Which task an answer belongs to: a prompt's is the task the prompt starts, whatever its session
//! had before; any other event's is its session's open task. And the experience hint, once a task.

use std::path::PathBuf;

use cortex_core::{Agent, Event, HookKind, Identifier, TaskOutcome, Ulid};
use cortex_store::{SessionRecord, TaskRecord};

use super::{experience_claim, hint_given, settle, task_of_answer};
use crate::claim::Claim;
use crate::once::ClaimedOnce;
use crate::test_store::{TestResult, writer};

/// An event of `hook` in session `s1` at 1,000.
fn event(hook: HookKind) -> TestResult<Event> {
    Ok(Event {
        event_id: Ulid::from_seed(1, b"answer"),
        ts_ms: 1_000,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("s1")?,
        parent_session_id: None,
        cwd: PathBuf::from("/repo"),
        repo_root: PathBuf::from("/repo"),
        worktree_root: None,
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: (hook == HookKind::Prompt).then(|| "fix it".to_owned()),
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// Session `s1`'s first task, stored as not held out, ended at 900; a prompt comes at 1,000 and
/// is not recorded yet, as when its answer is worked out. With every task held out (a share of 1)
/// the prompt's answer is in the task the prompt starts, held out, though the session has no open
/// task and its last one was not held out; with none held out (0), the same task, not held out.
/// Any other event's answer then is in no task.
#[test]
fn a_prompt_s_answer_is_in_the_task_it_starts() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut writer = writer(root.path())?;
    let first = Ulid::parse("01JC0000000000000000000000")?;
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: Identifier::new("s1")?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 100,
    })?;
    transaction.start_task(&TaskRecord {
        task_id: first,
        session_id: Identifier::new("s1")?,
        started_at_ms: 500,
        prompt_hash: None,
        prompt_excerpt: None,
        holdout: Some(false),
    })?;
    transaction.end_task(first, 900, TaskOutcome::Success)?;
    transaction.commit()?;
    let queries = writer.queries();
    let asked = event(HookKind::Prompt)?;
    let (task, is_held) = task_of_answer(queries, &asked, 1.0)?;
    assert!(task.is_some_and(|task| task != first));
    assert!(is_held);
    assert_eq!(task_of_answer(queries, &asked, 0.0)?, (task, false));
    assert_eq!(
        task_of_answer(queries, &event(HookKind::PostTool)?, 1.0)?,
        (None, false)
    );
    Ok(())
}

/// The experience hint is claimed once a task. With `history.experience` off nothing is claimed.
/// On, the first answer of task 1 claims it; a second answer of the same task while that claim
/// holds gets none, and settling that second answer (no hint given) leaves the first's claim. The
/// first was hinted: its claim is kept, and the task gets no second hint. Task 2's answer whose
/// work order carried no hint, or that claimed only another task's hint, gives its claim back, so a
/// later answer may carry it.
#[test]
fn a_task_is_given_the_experience_hint_once() {
    let experienced = ClaimedOnce::default();
    let (task, other) = (Ulid::from_seed(1, b"task"), Ulid::from_seed(2, b"task"));
    assert_eq!(experience_claim(false, (task, 1_000), &experienced), None);
    let first = experience_claim(true, (task, 1_000), &experienced);
    assert_eq!(first, Some(1_000));
    let second = experience_claim(true, (task, 1_000), &experienced);
    assert_eq!(second, None);
    settle(&experienced, (task, second), &[]);
    assert_eq!(experience_claim(true, (task, 1_000), &experienced), None);
    settle(&experienced, (task, first), &[Claim::Experienced(task)]);
    assert_eq!(experience_claim(true, (task, 1_000), &experienced), None);
    let theirs = experience_claim(true, (other, 2_000), &experienced);
    assert_eq!(theirs, Some(2_000));
    settle(&experienced, (other, theirs), &[Claim::Experienced(task)]);
    let again = experience_claim(true, (other, 2_000), &experienced);
    assert_eq!(again, Some(2_000));
    settle(&experienced, (other, again), &[]);
    assert_eq!(
        experience_claim(true, (other, 2_000), &experienced),
        Some(2_000)
    );
}

/// A work order's text, holding the experience hint or not.
fn work_order(is_hinted: bool) -> cortex_retrieve::Outcome {
    cortex_retrieve::Outcome::WorkOrder(
        cortex_retrieve::Rule::Agreement,
        cortex_retrieve::Rendered {
            text: "[cortex] Work order\n".to_owned(),
            tokens: 5,
            written: Vec::new(),
            shown: Vec::new(),
            facts: Vec::new(),
            is_hinted,
        },
    )
}

/// The claim is kept only for a hint in a text the agent is given: claimed, not held out, and a
/// work order whose text holds the hint. Each of the three alone, a work order without the hint,
/// a held-out task's, an answer that made no claim and a silent retrieval give it back.
#[test]
fn the_hint_is_given_only_in_a_text_given() {
    let silent = cortex_retrieve::Outcome::Silent(cortex_retrieve::Silence::NothingFound);
    assert!(hint_given(Some(1_000), false, &work_order(true)));
    assert!(!hint_given(Some(1_000), false, &work_order(false)));
    assert!(!hint_given(Some(1_000), true, &work_order(true)));
    assert!(!hint_given(None, false, &work_order(true)));
    assert!(!hint_given(Some(1_000), false, &silent));
}
