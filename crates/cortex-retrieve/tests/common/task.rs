//! A task built in the store by hand, for the tests of what a task did so far: its session and
//! task, files of one function, its reads and edits, and its warnings.

use cortex_core::{
    Agent, EventKind, EventMeta, FileFlags, Identifier, PromptExcerpt, RepoPath, ToolKind, Ulid,
    WarningKind,
};
use cortex_store::{EventRecord, NodeId, SessionRecord, TaskRecord, WarningRecord};

use super::{Repo, TestResult, function};

/// The session of the task.
pub const SESSION: &str = "s1";

/// Stores session [`SESSION`] with an open task, its prompt kept as `excerpt`; returns the task.
pub fn task(repo: &mut Repo, excerpt: Option<&str>) -> TestResult<Ulid> {
    let task = Ulid::from_seed(1, b"task");
    let session = Identifier::new(SESSION)?;
    let mut transaction = repo.writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: session.clone(),
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 1,
    })?;
    transaction.start_task(&TaskRecord {
        task_id: task,
        session_id: session,
        started_at_ms: 2,
        prompt_hash: None,
        prompt_excerpt: excerpt.map(PromptExcerpt::of),
        holdout: Some(false),
    })?;
    transaction.commit()?;
    Ok(task)
}

/// Stores a file at `path` with one function; returns its file node and the function's node.
pub fn file(repo: &mut Repo, path: &str) -> TestResult<(NodeId, NodeId)> {
    let nodes = repo.file(
        path,
        "def f():\n    return 1\n",
        &[function("f", 1, 2)],
        FileFlags::empty(),
    )?;
    match nodes.as_slice() {
        [file, symbol, ..] => Ok((*file, *symbol)),
        _ => Err("a file and its function".into()),
    }
}

/// Records that the task's `tool` acted on the file of `node` at `path` at `ts_ms`, succeeding as
/// `is_success` says.
pub fn acted(
    repo: &mut Repo,
    task: Ulid,
    (tool, path, node): (ToolKind, &str, NodeId),
    (ts_ms, is_success): (i64, bool),
) -> TestResult {
    let kind = if tool == ToolKind::Read {
        EventKind::Read
    } else {
        EventKind::Edit
    };
    let mut transaction = repo.writer.transaction()?;
    transaction.append_event(&EventRecord {
        task_id: Some(task),
        session_id: Some(Identifier::new(SESSION)?),
        ts_ms,
        kind,
        node_id: Some(node),
        path: Some(RepoPath::new(path)?),
        range: None,
        meta: EventMeta {
            tool: Some(tool),
            is_success: Some(is_success),
            ..EventMeta::default()
        },
    })?;
    transaction.commit()?;
    Ok(())
}

/// Records that the task was warned, at `at_ms`, of `kind` about `source`, naming `target`.
pub fn warned(
    repo: &mut Repo,
    task: Ulid,
    (kind, source, target): (WarningKind, NodeId, NodeId),
    at_ms: i64,
) -> TestResult {
    let mut transaction = repo.writer.transaction()?;
    transaction.record_warning(&WarningRecord {
        task_id: task,
        at_ms,
        kind,
        source,
        target,
        strength: 0.8,
    })?;
    transaction.commit()?;
    Ok(())
}
