//! Which task an answer belongs to: a prompt's is the task the prompt starts, whatever its session
//! had before; any other event's is its session's open task.

use std::path::PathBuf;

use cortex_core::{Agent, Event, HookKind, Identifier, TaskOutcome, Ulid};
use cortex_store::{SessionRecord, TaskRecord};

use super::task_of_answer;
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
