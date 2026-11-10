//! The narrow tests a whole-suite run gets, through the real binary as Claude Code runs it, on the
//! repository the captured payloads come from: once the task edited `calc/ops.py`, a run of the
//! whole suite (`python -m pytest -q`) is told the file's own tests and the command that runs only
//! them; a second whole-suite run in the task is told nothing; a run that names its tests is told
//! nothing; a task held out from Cortex, or a repository in tiny mode, is told nothing; and with
//! `hints.narrow_tests` left at its default, nothing is said at all.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::time::Duration;

use common::calc::{OPS_FIXED, S1, captured, prompt, repository, tests_run};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, wait_for};
use cortex_core::{EventKind, Identifier};
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

/// The captured passing test run, made a run of `command`.
fn suite_run(root: &Path, command: &str) -> TestResult<Value> {
    let mut run = tests_run(root, "post-tool-use-bash-test-passing.json")?;
    run["tool_input"]["command"] = Value::from(command);
    Ok(run)
}

/// Starts the daemon on `repo` with `config`, opens the captured session's task, edits
/// `calc/ops.py` in it, and waits until the store holds the edit.
fn edited(repo: &Path, home: &Path, config: &str) -> TestResult<Daemon> {
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), config)?;
    let daemon = Daemon::start(repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(repo, home)?.is_some()))?;
    given(&prompt(repo, "Make divide return a true quotient.")?, home)?;
    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    given(&captured("post-tool-use-edit.json", repo)?, home)?;
    let session = Identifier::new(S1)?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        let queries = reader.queries();
        let Some(task) = queries.open_task(&session)? else {
            return Ok(false);
        };
        let events = queries.events_of_task(task.record.task_id, 100)?;
        Ok(events.iter().any(|row| row.record.kind == EventKind::Edit))
    })?;
    Ok(daemon)
}

/// With the hint on, the first whole-suite run after the edit is told the edited file's own tests
/// and the command that runs only them, in the way the run was launched, its `-q` repeated; a run
/// naming its tests and a second whole-suite run are told nothing.
#[test]
fn a_whole_suite_run_is_told_the_narrow_tests() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = edited(
        &repo,
        home,
        "mode = \"medium\"\n[holdout]\nshare = 0\n[hints]\nnarrow_tests = true\n",
    )?;
    assert_eq!(
        given(
            &suite_run(&repo, "python -m pytest tests/test_ops.py")?,
            home
        )?,
        None
    );
    assert_eq!(
        given(&suite_run(&repo, "python -m pytest -q")?, home)?.as_deref(),
        Some(
            "[cortex] Tests for the files changed in this task: tests/test_ops.py \
             (`python -m pytest -q tests/test_ops.py`).\n"
        )
    );
    assert_eq!(
        given(&suite_run(&repo, "python -m pytest -q")?, home)?,
        None
    );
    drop(daemon);
    Ok(())
}

/// With `hints.narrow_tests` left at its default, the whole-suite run is told nothing.
#[test]
fn the_narrow_tests_are_off_by_default() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = edited(&repo, home, "mode = \"medium\"\n[holdout]\nshare = 0\n")?;
    assert_eq!(
        given(&suite_run(&repo, "python -m pytest -q")?, home)?,
        None
    );
    drop(daemon);
    Ok(())
}

/// A task held out from Cortex is told nothing, whatever the setting: every task is held out when
/// the share is one.
#[test]
fn a_held_out_task_is_told_nothing() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = edited(
        &repo,
        home,
        "mode = \"medium\"\n[holdout]\nshare = 1\n[hints]\nnarrow_tests = true\n",
    )?;
    assert_eq!(
        given(&suite_run(&repo, "python -m pytest -q")?, home)?,
        None
    );
    drop(daemon);
    Ok(())
}

/// A repository in tiny mode is told nothing, as Cortex says nothing there.
#[test]
fn a_tiny_repository_is_told_nothing() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = edited(
        &repo,
        home,
        "mode = \"tiny\"\n[holdout]\nshare = 0\n[hints]\nnarrow_tests = true\n",
    )?;
    assert_eq!(
        given(&suite_run(&repo, "python -m pytest -q")?, home)?,
        None
    );
    drop(daemon);
    Ok(())
}
