//! Which position a session keeps on a file as its reads and edits come in.

use std::path::PathBuf;

use cortex_coord::{PositionMode, Timing};
use cortex_core::{
    Agent, Event, HookKind, Identifier, RepoPath, TaskOutcome, ToolKind, ToolResult, ToolTarget,
    Ulid,
};
use cortex_store::{NodeId, SessionRecord, TaskRecord, Writer};

use super::keep;
use crate::test_store::{TestResult, add_file, writer};

/// Holds short enough to count by hand: a read holds 10 s, an edit 60 s after the session's last
/// edit or 15 s after its task ended, whichever comes first.
const TIMING: Timing = Timing {
    read_ms: 10_000,
    edit_ms: 60_000,
    edit_after_task_ms: 15_000,
    claim_ms: 30_000,
    live_ms: 30_000,
    same_file_ms: 5_000,
    handoff_idle_ms: 60_000,
};

/// Session `s1`'s call to `tool` on `paths` at `at_ms`, seen at `hook`, ending as `is_success`
/// says.
fn call(
    hook: HookKind,
    tool: ToolKind,
    paths: &[&str],
    (at_ms, is_success): (i64, bool),
) -> TestResult<Event> {
    let mut paths = paths.iter().map(|path| RepoPath::new(path));
    let path = paths.next().transpose()?;
    let more_paths = paths.collect::<Result<Vec<_>, _>>()?;
    Ok(Event {
        event_id: Ulid::from_seed(u64::try_from(at_ms)?, b"call"),
        ts_ms: at_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new("s1")?,
        parent_session_id: None,
        cwd: PathBuf::from("/repo"),
        repo_root: PathBuf::from("/repo"),
        worktree_root: None,
        hook,
        source: None,
        tool: Some(tool),
        tool_target: Some(ToolTarget {
            path,
            more_paths,
            ..ToolTarget::default()
        }),
        tool_result: (hook == HookKind::PostTool).then_some(ToolResult {
            exit_code: None,
            is_success,
            is_interrupted: false,
            output_bytes: None,
            counts: None,
            failing: None,
        }),
        prompt_text: None,
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// A successful `tool` call by `s1` on `paths` at `at_ms`, after it ran.
fn ran(tool: ToolKind, paths: &[&str], at_ms: i64) -> TestResult<Event> {
    call(HookKind::PostTool, tool, paths, (at_ms, true))
}

/// Keeps the positions `event` gives in `writer`'s store.
fn given(writer: &mut Writer, event: &Event) -> TestResult {
    let mut transaction = writer.transaction()?;
    keep(&mut transaction, event, &TIMING)?;
    transaction.commit()?;
    Ok(())
}

/// The position `s1` holds on `node`: its mode, since and last time.
fn held(writer: &Writer, node: NodeId) -> TestResult<Option<(PositionMode, i64, i64)>> {
    let row = writer
        .queries()
        .position_of(&Identifier::new("s1")?, node)?;
    Ok(row.map(|row| (row.record.mode, row.record.since_ms, row.record.last_ms)))
}

/// Reads at 1 s and 2 s hold until 12 s, one read since 1 s. An edit at 3 s holds until 63 s and
/// replaces it, since 3 s. A read at 4 s would hold until 14 s only: the edit stays. Another edit
/// at 5 s holds until 65 s: the edit is now last at 5 s, still since 3 s.
#[test]
fn a_session_keeps_the_position_that_holds_longest() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut writer = writer(root.path())?;
    let node = add_file(&mut writer, "a.py", 1)?;
    given(&mut writer, &ran(ToolKind::Read, &["a.py"], 1_000)?)?;
    assert_eq!(
        held(&writer, node)?,
        Some((PositionMode::Read, 1_000, 1_000))
    );
    given(&mut writer, &ran(ToolKind::Read, &["a.py"], 2_000)?)?;
    assert_eq!(
        held(&writer, node)?,
        Some((PositionMode::Read, 1_000, 2_000))
    );
    given(&mut writer, &ran(ToolKind::Edit, &["a.py"], 3_000)?)?;
    assert_eq!(
        held(&writer, node)?,
        Some((PositionMode::Edit, 3_000, 3_000))
    );
    given(&mut writer, &ran(ToolKind::Read, &["a.py"], 4_000)?)?;
    assert_eq!(
        held(&writer, node)?,
        Some((PositionMode::Edit, 3_000, 3_000))
    );
    given(&mut writer, &ran(ToolKind::MultiEdit, &["a.py"], 5_000)?)?;
    assert_eq!(
        held(&writer, node)?,
        Some((PositionMode::Edit, 3_000, 5_000))
    );
    Ok(())
}

/// `s1`'s task ran from 0.5 s to 2 s and edited `a.py` at 1 s: the edit holds until 17 s, 15 s
/// after the task ended, not until 61 s. A read at 10 s holds until 20 s and replaces it.
#[test]
fn an_edit_whose_task_ended_gives_way_to_a_later_read() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut writer = writer(root.path())?;
    let node = add_file(&mut writer, "a.py", 1)?;
    let task_id = Ulid::parse("01JC0000000000000000000000")?;
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
        task_id,
        session_id: Identifier::new("s1")?,
        started_at_ms: 500,
        prompt_hash: None,
        prompt_excerpt: None,
        holdout: Some(false),
    })?;
    transaction.commit()?;
    given(&mut writer, &ran(ToolKind::Edit, &["a.py"], 1_000)?)?;
    let mut transaction = writer.transaction()?;
    transaction.end_task(task_id, 2_000, TaskOutcome::Success)?;
    transaction.commit()?;
    given(&mut writer, &ran(ToolKind::Read, &["a.py"], 10_000)?)?;
    assert_eq!(
        held(&writer, node)?,
        Some((PositionMode::Read, 10_000, 10_000))
    );
    Ok(())
}

/// A failed edit, a read about to run, a shell command and a read of a file the store does not
/// hold move nothing; a patch of two stored files takes both.
#[test]
fn only_a_successful_read_or_edit_of_a_stored_file_takes_a_position() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut writer = writer(root.path())?;
    let a = add_file(&mut writer, "a.py", 1)?;
    let b = add_file(&mut writer, "b.py", 1)?;
    let failed = call(
        HookKind::PostTool,
        ToolKind::Edit,
        &["a.py"],
        (1_000, false),
    )?;
    given(&mut writer, &failed)?;
    let before = call(HookKind::PreTool, ToolKind::Read, &["a.py"], (1_000, true))?;
    given(&mut writer, &before)?;
    given(&mut writer, &ran(ToolKind::Shell, &["a.py"], 1_000)?)?;
    given(&mut writer, &ran(ToolKind::Read, &["c.py"], 1_000)?)?;
    assert_eq!(held(&writer, a)?, None);
    assert_eq!(writer.queries().positions_since(0)?.len(), 0);
    given(
        &mut writer,
        &ran(ToolKind::ApplyPatch, &["a.py", "b.py"], 2_000)?,
    )?;
    assert_eq!(held(&writer, a)?, Some((PositionMode::Edit, 2_000, 2_000)));
    assert_eq!(held(&writer, b)?, Some((PositionMode::Edit, 2_000, 2_000)));
    Ok(())
}
