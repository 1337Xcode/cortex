//! A companion warning, through the real binary as Claude Code runs it, on a repository whose
//! history ties two files together: `calc/ops.py` and `docs/ops.md` changed in the same four
//! commits and never apart. An edit of `calc/ops.py` is told of `docs/ops.md`; once the task edits
//! it and ends, the store marks the warning resolved.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::time::Duration;

use common::calc::{S1, captured, prompt, repository};
use common::git::{commit, git};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, wait_for};
use cortex_core::{Identifier, WarningKind};
use cortex_store::Reader;
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

/// The captured write made a write of `text` to `path` under `root`.
fn write(root: &Path, path: &str, text: &str) -> TestResult<Value> {
    std::fs::write(root.join(path), text)?;
    let native = Value::from(root.join(path).to_string_lossy().into_owned());
    let mut write = captured("post-tool-use-write.json", root)?;
    write["tool_input"]["file_path"] = native.clone();
    write["tool_input"]["content"] = Value::from(text);
    write["tool_response"]["filePath"] = native;
    write["tool_response"]["content"] = Value::from(text);
    write["tool_response"]["type"] = Value::from("update");
    Ok(write)
}

/// The warnings the store holds of the captured session's task: each one's kind, the path it
/// named and its label.
fn warnings(root: &Path) -> TestResult<Vec<(WarningKind, String, Option<bool>)>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let queries = reader.queries();
    let Some(task) = queries
        .tasks_of_session(&Identifier::new(S1)?)?
        .first()
        .cloned()
    else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for row in queries.warnings_of_task(task.record.task_id)? {
        let (_, path) = queries
            .node_file(row.record.target)?
            .ok_or("a named file")?;
        found.push((row.record.kind, path.as_str().to_owned(), row.resolved));
    }
    Ok(found)
}

/// Four commits change `calc/ops.py` and `docs/ops.md` together, so each changed in all four of the
/// other's commits: confidence 1 from either side, 4 co-changes. The task first reads its tests, so
/// the warning is the companion alone: "4 of 4 changes". The task then writes `docs/ops.md` and
/// stops; its end finds the file edited after the warning and marks the warning resolved.
#[test]
fn a_companion_is_named_and_resolved_once_edited() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    std::fs::create_dir_all(repo.join("docs"))?;
    git(&repo, home, &["init", "-q"])?;
    let mut ops = std::fs::read_to_string(repo.join("calc/ops.py"))?;
    let mut docs = String::from("# Arithmetic helpers\n");
    for number in 1..=4 {
        ops.push_str(&format!("\n\ndef f{number}():\n    return {number}\n"));
        docs.push_str(&format!("\n`f{number}` returns {number}.\n"));
        std::fs::write(repo.join("calc/ops.py"), &ops)?;
        std::fs::write(repo.join("docs/ops.md"), &docs)?;
        let paths: &[&str] = if number == 1 {
            &["."]
        } else {
            &["calc/ops.py", "docs/ops.md"]
        };
        commit(&repo, home, &format!("change {number}"), paths)?;
    }
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(
        repo.join(".cortex/config.toml"),
        "mode = \"medium\"\n[holdout]\nshare = 0\n",
    )?;
    let daemon = Daemon::start(&repo, home)?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(&repo, home)?.is_some_and(|graph| graph.has_cochange)))?;
    given(&prompt(&repo, "Let f1 return one more.")?, home)?;
    let session = Identifier::new(S1)?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        Ok(reader.queries().open_task(&session)?.is_some())
    })?;
    let tests = std::fs::read_to_string(repo.join("tests/test_ops.py"))?;
    given(&write(&repo, "tests/test_ops.py", &tests)?, home)?;

    let changed = ops.replace("    return 1\n", "    return 2\n");
    assert_eq!(
        given(&write(&repo, "calc/ops.py", &changed)?, home)?.as_deref(),
        Some(
            "[cortex] Edits to calc/ops.py usually come with docs/ops.md (4 of 4 changes); not \
             touched yet.\n"
        )
    );
    wait_for(|| Ok(warnings(&repo)?.len() == 1))?;

    let documented = docs.replace("returns 1.", "returns 2.");
    given(&write(&repo, "docs/ops.md", &documented)?, home)?;
    given(&captured("stop.json", &repo)?, home)?;
    wait_for(|| Ok(warnings(&repo)?.iter().all(|(_, _, label)| label.is_some())))?;
    assert_eq!(
        warnings(&repo)?,
        [(WarningKind::CoChange, "docs/ops.md".to_owned(), Some(true))]
    );
    Ok(())
}
