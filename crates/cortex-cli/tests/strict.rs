//! Strict mode at a stop, through the real binary as Claude Code runs it, on the repository the
//! captured payloads come from (pinned to medium mode): with `strict_mode` on, once the task's
//! edit changed `add`'s signature, the stop is given the files that still call it, once a task, and
//! is not recorded: the agent goes on, and its task with it, until the next stop ends the task. With
//! it off, as by default, the same stop is given nothing and ends the task. An answer that never
//! reaches its hook gives back what it claimed: a recap worked out too late is given to the next
//! start of the same compaction.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::calc::{OPS, S1, captured, prompt, repository};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, wait_for};
use cortex_core::ipc::{RequestBody, ResponseBody, Role};
use cortex_core::{
    Agent, Event, EventKind, HookKind, Identifier, RepoId, StartSource, Ulid, UserFolder,
    canonical_root,
};
use cortex_hook::ask;
use cortex_store::Reader;
use serde_json::Value;

/// What the stop is told, by hand: the edit of `add` was warned of its calls in `main.py`, then in
/// `tests/test_ops.py`: the later first.
const REASON: &str = "[cortex] Strict mode: these files call a signature this task changed and \
    were not read or edited since the warning: tests/test_ops.py, main.py.\n";

/// The configuration with strict mode on, in medium mode, no task held out.
const STRICT: &str = "mode = \"medium\"\nstrict_mode = true\n[holdout]\nshare = 0\n";

/// Runs `cortex hook` on `payload` as Claude Code does, and returns what it gave the agent.
fn given(payload: &Value, home: &Path) -> TestResult<Option<String>> {
    let event = payload["hook_event_name"]
        .as_str()
        .ok_or("no event name")?
        .to_owned();
    let call = hook(
        &["--agent", "claude", "--event", &event],
        serde_json::to_vec(payload)?,
        Duration::ZERO,
        home,
    )?;
    call.context(&event)
}

/// Runs a task on the captured repository with `config` as its configuration: a prompt, then the
/// edit of `add`'s header, told of its callers, its warnings stored. Returns the stop's payload
/// and the home folder, and keeps the daemon running.
fn edited_task(config: &str) -> TestResult<(Value, tempfile::TempDir, Daemon)> {
    let (home, repo) = repository()?;
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), config)?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home.path())?.is_some()))?;
    given(&prompt(&repo, "Let add take a third number.")?, home.path())?;
    let session = Identifier::new(S1)?;
    let store = || Reader::open(&repo.join(".cortex"), 0);
    wait_for(|| Ok(store()?.queries().open_task(&session)?.is_some()))?;
    let (old, new) = ("def add(a, b):", "def add(a, b, c=0):");
    std::fs::write(repo.join("calc/ops.py"), OPS.replace(old, new))?;
    let mut edit = captured("post-tool-use-edit.json", &repo)?;
    edit["tool_input"]["old_string"] = Value::from(old);
    edit["tool_input"]["new_string"] = Value::from(new);
    let response = &mut edit["tool_response"];
    response["oldString"] = Value::from(old);
    response["newString"] = Value::from(new);
    response["originalFile"] = Value::from(OPS);
    response["structuredPatch"] = serde_json::json!([{
        "oldStart": 4, "oldLines": 1, "newStart": 4, "newLines": 1,
        "lines": [format!("-{old}"), format!("+{new}")],
    }]);
    let warned = given(&edit, home.path())?.ok_or("the edit's warnings")?;
    assert!(warned.contains("add's signature changed"), "{warned}");
    wait_for(|| {
        let reader = store()?;
        let queries = reader.queries();
        let task = queries.open_task(&session)?.ok_or("the task")?;
        Ok(queries.warnings_of_task(task.record.task_id)?.len() == 3)
    })?;
    Ok((captured("stop.json", &repo)?, home, daemon))
}

/// The captured session's stops the store holds, and whether it still has an open task.
fn stops(home: &Path) -> TestResult<(usize, bool)> {
    let reader = Reader::open(&home.join("repo").join(".cortex"), 0)?;
    let queries = reader.queries();
    let session = Identifier::new(S1)?;
    let events = queries.events_of_session(&session)?;
    let stops = events
        .iter()
        .filter(|row| row.record.kind == EventKind::Stop)
        .count();
    Ok((stops, queries.open_task(&session)?.is_some()))
}

/// With strict mode on, the stop is given the callers not followed up, once in the task: the
/// stop given its reason is not recorded and leaves the task open, so the second stop is asked of
/// the same task, gets nothing, and is the one stop recorded, which ends the task.
#[test]
fn strict_mode_gives_a_stop_its_reason_once() -> TestResult {
    let (stop, home, _daemon) = edited_task(STRICT)?;
    assert_eq!(given(&stop, home.path())?.as_deref(), Some(REASON));
    assert_eq!(given(&stop, home.path())?, None);
    wait_for(|| Ok(!stops(home.path())?.1))?;
    assert_eq!(stops(home.path())?, (1, false));
    Ok(())
}

/// With strict mode off, the same stop is given nothing, and ends the task.
#[test]
#[ignore = "timing under load: its premise (the edit re-indexed within the edit's budget) needs an idle machine; `cargo xtask verify --full` runs it"]
fn without_strict_mode_a_stop_is_given_nothing() -> TestResult {
    let (stop, home, _daemon) = edited_task("mode = \"medium\"\n[holdout]\nshare = 0\n")?;
    assert_eq!(given(&stop, home.path())?, None);
    wait_for(|| Ok(!stops(home.path())?.1))?;
    assert_eq!(stops(home.path())?, (1, false));
    Ok(())
}

/// A stop's request carrying an event of the captured session at `hook`, made now.
fn stop_request(root: &Path, hook: HookKind, answer_by_ms: Option<i64>) -> TestResult<RequestBody> {
    Ok(RequestBody::StopReason {
        event: Box::new(event_now(root, hook)?),
        answer_by_ms,
    })
}

/// An event of the captured session at `hook`, made now.
fn event_now(root: &Path, hook: HookKind) -> TestResult<Event> {
    let root = canonical_root(root)?;
    let ts_ms = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let event = Event {
        event_id: Ulid::from_seed(u64::try_from(ts_ms)?, b"strict"),
        ts_ms,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new(S1)?,
        parent_session_id: None,
        cwd: root.clone(),
        repo_root: root,
        hook,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: None,
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    };
    Ok(event)
}

/// A stop's request whose event is of another hook is refused and records nothing; one whose
/// reason is ready only after its hook stopped reading gives nothing, and the stop, which then
/// happened, is recorded and ends the task.
#[test]
fn a_late_stop_is_recorded_and_a_mismatched_one_refused() -> TestResult {
    let (_stop, home, _daemon) = edited_task(STRICT)?;
    let repo = home.path().join("repo");
    let files = UserFolder::in_home(home.path()).run_files(&RepoId::of(&canonical_root(&repo)?));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mismatched = stop_request(&repo, HookKind::Prompt, None)?;
    assert!(matches!(
        ask(&files, Role::Hook, mismatched, deadline)?.body,
        ResponseBody::Error { .. }
    ));
    let late = stop_request(&repo, HookKind::Stop, Some(0))?;
    assert_eq!(
        ask(&files, Role::Hook, late, deadline)?.body,
        ResponseBody::Reply {
            text: None,
            to_user: None
        }
    );
    wait_for(|| Ok(!stops(home.path())?.1))?;
    assert_eq!(stops(home.path())?, (1, false));
    Ok(())
}

/// The task was edited and warned; its session's context is compacted. The start that follows is
/// answered too late, gets nothing, and gives its claim on the compaction back: the next start of
/// the same compaction, answered in time, gets the recap; a third gets nothing, the compaction
/// being recapped.
#[test]
fn a_recap_that_came_too_late_is_given_to_the_next_start() -> TestResult {
    let (_stop, home, _daemon) = edited_task(STRICT)?;
    let repo = home.path().join("repo");
    let files = UserFolder::in_home(home.path()).run_files(&RepoId::of(&canonical_root(&repo)?));
    let deadline = Instant::now() + Duration::from_secs(10);
    let compacted = RequestBody::Event {
        event: Box::new(event_now(&repo, HookKind::Compact)?),
    };
    assert_eq!(
        ask(&files, Role::Hook, compacted, deadline)?.body,
        ResponseBody::Ack
    );
    let start = |answer_by_ms| -> TestResult<RequestBody> {
        let event = Event {
            source: Some(StartSource::Compact),
            ..event_now(&repo, HookKind::SessionStart)?
        };
        Ok(RequestBody::Primer {
            event: Box::new(event),
            answer_by_ms,
        })
    };
    let text = |answer_by_ms| -> TestResult<Option<String>> {
        match ask(&files, Role::Hook, start(answer_by_ms)?, deadline)?.body {
            ResponseBody::Reply { text, .. } => Ok(text),
            other => Err(format!("{other:?}").into()),
        }
    };
    assert_eq!(text(Some(0))?, None);
    let recap = text(None)?.ok_or("the recap, given back")?;
    assert!(
        recap.starts_with("[cortex] Before the compaction, the task in progress:\n"),
        "{recap}"
    );
    assert_eq!(text(None)?, None);
    Ok(())
}
