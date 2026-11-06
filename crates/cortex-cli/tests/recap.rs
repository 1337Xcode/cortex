//! The recap after a compaction, through the real binary as Claude Code runs it, on the repository
//! the captured payloads come from (pinned to medium mode): once the session's task edited
//! `calc/ops.py` and was warned of its callers and tests, the start that follows the captured
//! compaction is told what the task edited and what it was warned of, once. The same start again
//! and the compaction's end are told nothing; the next compaction is recapped again.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::time::Duration;

use common::calc::{OPS, S1, captured, prompt, repository};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, wait_for};
use cortex_core::{EventKind, HookKind, Identifier, estimate_tokens};
use cortex_store::{Queries, Reader};
use serde_json::Value;

/// What the start after the captured compaction is told, by hand: the task edited `calc/ops.py`;
/// its edit was warned of the calls to `add` in `main.py` and `tests/test_ops.py`, then of the
/// tests: the latest first, each file once.
const RECAP: &str = "[cortex] Before the compaction, the task in progress:\n\
    - edited calc/ops.py\n\
    - was warned of tests/test_ops.py, the tests of calc/ops.py; not read or edited since\n\
    - was warned of main.py, which calls a signature this task changed; not read or edited since\n";

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

/// Reads the store of the repository at `root` with `read`.
fn read<T>(root: &Path, read: impl FnOnce(Queries<'_>) -> TestResult<T>) -> TestResult<T> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    read(reader.queries())
}

/// How many of the captured session's events of `kind` the store holds.
fn stored(root: &Path, kind: EventKind) -> TestResult<usize> {
    read(root, |queries| {
        let events = queries.events_of_session(&Identifier::new(S1)?)?;
        Ok(events.iter().filter(|row| row.record.kind == kind).count())
    })
}

/// How many texts given at a session's start the store holds for the captured session.
fn starts_given(root: &Path) -> TestResult<usize> {
    read(root, |queries| {
        let given = queries.injections_of_session(&Identifier::new(S1)?)?;
        Ok(given
            .iter()
            .filter(|row| row.record.hook == HookKind::SessionStart)
            .count())
    })
}

/// The start after each compaction is recapped once, within its cap.
#[test]
fn the_start_after_a_compaction_is_recapped_once() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), "mode = \"medium\"\n")?;
    let daemon = Daemon::start(&repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home)?.is_some()))?;
    given(&prompt(&repo, "Let add take a third number.")?, home)?;
    let session = Identifier::new(S1)?;
    wait_for(|| read(&repo, |queries| Ok(queries.open_task(&session)?.is_some())))?;

    // The edit of `add`'s header, as the warnings test makes it.
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
    let warned = given(&edit, home)?.ok_or("the edit's warnings")?;
    assert!(warned.contains("add's signature changed"), "{warned}");
    wait_for(|| {
        read(&repo, |queries| {
            let task = queries.open_task(&session)?.ok_or("the task")?;
            Ok(queries.warnings_of_task(task.record.task_id)?.len() == 3)
        })
    })?;
    wait_for(|| Ok(stored(&repo, EventKind::Edit)? == 1))?;

    let start = captured("session-start-compact.json", &repo)?;
    let end = captured("post-compact-manual.json", &repo)?;
    let compaction = captured("pre-compact-manual.json", &repo)?;
    // The start comes right after the compaction, as an agent sends it, before the writer may
    // have recorded the compaction.
    given(&compaction, home)?;
    let recap = given(&start, home)?;
    assert_eq!(recap.as_deref(), Some(RECAP));
    assert!(estimate_tokens(RECAP) <= 500);
    assert_eq!(given(&start, home)?, None);
    assert_eq!(given(&end, home)?, None);
    wait_for(|| Ok(starts_given(&repo)? == 1))?;

    // The next compaction gets a recap of its own.
    given(&compaction, home)?;
    assert_eq!(given(&start, home)?.as_deref(), Some(RECAP));
    assert_eq!(given(&start, home)?, None);
    wait_for(|| Ok(starts_given(&repo)? == 2))?;
    wait_for(|| Ok(stored(&repo, EventKind::Compact)? == 2))?;
    Ok(())
}
