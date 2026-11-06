//! Every text Cortex gives a task, through the real binary as Claude Code runs it, on the
//! repository the captured payloads come from (pinned to medium mode, strict mode on): a read's
//! hint, an edit's warnings, the recap after a compaction and a stop's reason are each recorded
//! as the task's, with the hash and the tokens of the text the hook printed, so receipts count
//! them; a held-out task is given none of them, and nothing of them is recorded.
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
use cortex_core::{ContentHash, EventKind, HookKind, Identifier, Ulid, estimate_tokens};
use cortex_store::{InjectionRecord, Reader};
use serde_json::Value;

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

/// What the store under `repo` says of the captured session: its open task, and every text
/// recorded as given to it other than its prompt's work order.
fn recorded(repo: &Path) -> TestResult<(Option<Ulid>, Vec<InjectionRecord>)> {
    let reader = Reader::open(&repo.join(".cortex"), 0)?;
    let queries = reader.queries();
    let session = Identifier::new(S1)?;
    let task = queries.open_task(&session)?.map(|task| task.record.task_id);
    let texts = queries
        .injections_of_session(&session)?
        .into_iter()
        .map(|row| row.record)
        .filter(|record| record.hook != HookKind::Prompt)
        .collect();
    Ok((task, texts))
}

/// How many of the captured session's events of `kind` the store holds.
fn stored(repo: &Path, kind: EventKind) -> TestResult<usize> {
    let reader = Reader::open(&repo.join(".cortex"), 0)?;
    let events = reader.queries().events_of_session(&Identifier::new(S1)?)?;
    Ok(events.iter().filter(|row| row.record.kind == kind).count())
}

/// Runs a task on the captured repository with `holdout.share` at `share`: a prompt, the
/// session's whole read of `calc/ops.py` and then a read of some of its lines, the edit of `add`'s
/// header, a compaction and the start after it, and a stop. Returns what each of the four gave the
/// agent, in that order, the repository and the home folder, and keeps the daemon running.
fn run_task(
    share: &str,
) -> TestResult<(
    [Option<String>; 4],
    tempfile::TempDir,
    std::path::PathBuf,
    Daemon,
)> {
    let (home, repo) = repository()?;
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(
        repo.join(".cortex/config.toml"),
        format!("mode = \"medium\"\nstrict_mode = true\n[holdout]\nshare = {share}\n"),
    )?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home.path())?.is_some()))?;
    given(&prompt(&repo, "Let add take a third number.")?, home.path())?;
    wait_for(|| Ok(recorded(&repo)?.0.is_some()))?;

    given(&captured("post-tool-use-read.json", &repo)?, home.path())?;
    wait_for(|| Ok(stored(&repo, EventKind::Read)? == 1))?;
    let mut range = captured("pre-tool-use-read-range.json", &repo)?;
    range["session_id"] = Value::from(S1);
    let hint = given(&range, home.path())?;

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
    let warnings = given(&edit, home.path())?;
    wait_for(|| Ok(stored(&repo, EventKind::Edit)? == 1))?;

    given(&captured("pre-compact-manual.json", &repo)?, home.path())?;
    let recap = given(&captured("session-start-compact.json", &repo)?, home.path())?;
    let stop = given(&captured("stop.json", &repo)?, home.path())?;
    Ok(([hint, warnings, recap, stop], home, repo, daemon))
}

/// A task that is not held out is given the four texts, and each is recorded as given to it, at
/// its hook, with the hash and the tokens of the text the hook printed.
#[test]
fn every_text_a_task_is_given_is_recorded() -> TestResult {
    let (texts, _home, repo, _daemon) = run_task("0")?;
    let hooks = [
        HookKind::PreTool,
        HookKind::PostTool,
        HookKind::SessionStart,
        HookKind::Stop,
    ];
    let mut expected = Vec::new();
    for (hook, text) in hooks.into_iter().zip(&texts) {
        let text = text.as_deref().ok_or(format!("no text at {hook:?}"))?;
        expected.push((
            hook,
            ContentHash::of(text.as_bytes()),
            u32::try_from(estimate_tokens(text))?,
        ));
    }
    let task = recorded(&repo)?.0.ok_or("the task")?;
    wait_for(|| Ok(recorded(&repo)?.1.len() == 4))?;
    let (_, records) = recorded(&repo)?;
    let kept: Vec<(HookKind, ContentHash, u32)> = records
        .iter()
        .filter(|record| record.task_id == Some(task) && record.was_delivered())
        .filter_map(|record| Some((record.hook, record.text_hash?, record.tokens)))
        .collect();
    assert_eq!(kept, expected);
    Ok(())
}

/// A held-out task is given none of the four texts, and none is recorded: the comparison between
/// held-out tasks and the rest holds out everything Cortex would say to a task.
#[test]
fn a_held_out_task_is_given_no_text() -> TestResult {
    let (texts, _home, repo, _daemon) = run_task("1")?;
    assert_eq!(texts, [None, None, None, None]);
    // The stop, given nothing, ended the task once recorded: what the store holds is complete.
    wait_for(|| Ok(stored(&repo, EventKind::Stop)? == 1))?;
    assert!(recorded(&repo)?.1.is_empty());
    Ok(())
}
