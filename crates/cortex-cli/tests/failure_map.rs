//! A failed check's map, through the real binary as Claude Code runs it, on the repository the
//! captured payloads come from: a unittest run whose traceback runs through `divide` and its test,
//! in files the task has not read, is given those lines with their code as it is on disk,
//! innermost first. With `hints.runtime_grounding` off, the same failure is given nothing; and a
//! failure whose frames are all outside the repository is given nothing either. The prompt after
//! the failure ranks the failing code higher than it would without it.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::time::Duration;

use common::calc::{S1, prompt, repository, tests_run};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, wait_for};
use cortex_core::{HookKind, Identifier};
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

/// The repository's root as Python prints it in a traceback: the canonical path, without the
/// Windows verbatim prefix.
fn printed_root(root: &Path) -> TestResult<String> {
    let canonical = root.canonicalize()?.to_string_lossy().into_owned();
    Ok(canonical
        .strip_prefix(r"\\?\")
        .unwrap_or(&canonical)
        .to_owned())
}

/// The captured failed test run, made a run of `tests.test_ops` that failed in `divide`, from the
/// repository's root after a `cd` to it, as agents write it (so the line's exit may be the `cd`'s
/// as well as the tests'): its traceback names `tests/test_ops.py`, line 14, then `calc/ops.py`,
/// line 13, both by their full paths under `at`, as unittest prints them, with each line's code.
fn failed_run(root: &Path, at: &str) -> TestResult<Value> {
    let separator = std::path::MAIN_SEPARATOR;
    let file = |path: &str| {
        format!(
            "{at}{separator}{}",
            path.replace('/', &separator.to_string())
        )
    };
    let output = format!(
        "Exit code 1\nE\n\
         ======================================================================\n\
         ERROR: test_divide (tests.test_ops.OpsTest.test_divide)\n\
         ----------------------------------------------------------------------\n\
         Traceback (most recent call last):\n  \
         File \"{tests}\", line 14, in test_divide\n    \
         self.assertEqual(divide(7, 2), 3.5)\n  \
         File \"{ops}\", line 13, in divide\n    \
         return a // b\n\
         ZeroDivisionError: integer division or modulo by zero\n\n\
         ----------------------------------------------------------------------\n\
         Ran 3 tests in 0.001s\n\nFAILED (errors=1)",
        tests = file("tests/test_ops.py"),
        ops = file("calc/ops.py"),
    );
    let mut run = tests_run(root, "post-tool-use-failure-bash-test.json")?;
    let command = format!(
        "cd \"{}\" && python -m unittest tests.test_ops -q",
        root.display()
    );
    run["tool_input"]["command"] = Value::from(command);
    run["error"] = Value::from(output);
    Ok(run)
}

/// Starts the daemon on `repo` with `config` and opens the captured session's task with a prompt.
fn started(repo: &Path, home: &Path, config: &str) -> TestResult<Daemon> {
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), config)?;
    let daemon = Daemon::start(repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(repo, home)?.is_some()))?;
    given(&prompt(repo, "Make divide return a true quotient.")?, home)?;
    let session = Identifier::new(S1)?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        Ok(reader.queries().open_task(&session)?.is_some())
    })?;
    Ok(daemon)
}

/// With the map on, the failed run is given the two lines it ran through, `divide`'s first, each
/// with its code read from disk and the definition holding it. A failure whose frames are all
/// outside the repository (an installed package's) is given nothing.
#[test]
fn a_failed_check_is_given_the_lines_it_ran_through() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = started(
        &repo,
        home,
        "mode = \"medium\"\n[holdout]\nshare = 0\n[hints]\nruntime_grounding = true\n",
    )?;
    let at = printed_root(&repo)?;
    assert_eq!(
        given(&failed_run(&repo, &at)?, home)?.as_deref(),
        Some(
            "[cortex] The failing test test_divide (tests.test_ops.OpsTest.test_divide) runs \
             through these lines, as they are now:\n  \
             calc/ops.py:13 in divide: return a // b\n  \
             tests/test_ops.py:14 in test_divide: self.assertEqual(divide(7, 2), 3.5)\n"
        )
    );
    let outside = tempfile::tempdir()?;
    let elsewhere = printed_root(outside.path())?;
    assert_eq!(given(&failed_run(&repo, &elsewhere)?, home)?, None);
    drop(daemon);
    Ok(())
}

/// With the map on, a held-out task (every task held out) and a tiny repository's task are given
/// nothing for the same failed run.
#[test]
fn a_held_out_or_tiny_task_gets_no_map() -> TestResult {
    for config in [
        "mode = \"medium\"\n[holdout]\nshare = 1\n[hints]\nruntime_grounding = true\n",
        "mode = \"tiny\"\n[holdout]\nshare = 0\n[hints]\nruntime_grounding = true\n",
    ] {
        let (home, repo) = repository()?;
        let home = home.path();
        let daemon = started(&repo, home, config)?;
        let at = printed_root(&repo)?;
        assert_eq!(given(&failed_run(&repo, &at)?, home)?, None, "{config}");
        drop(daemon);
    }
    Ok(())
}

/// With `hints.runtime_grounding` left at its default, the same failed run is given nothing.
#[test]
fn the_map_is_off_by_default() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = started(&repo, home, "mode = \"medium\"\n[holdout]\nshare = 0\n")?;
    let at = printed_root(&repo)?;
    assert_eq!(given(&failed_run(&repo, &at)?, home)?, None);
    drop(daemon);
    Ok(())
}

/// The files the retrieval of the captured session's latest prompt ranked, best first, as the
/// store recorded them, whether the gate gave them or not.
fn ranked_by_latest_prompt(repo: &Path) -> TestResult<Vec<String>> {
    let reader = Reader::open(&repo.join(".cortex"), 0)?;
    let queries = reader.queries();
    let latest = queries
        .injections_of_session(&Identifier::new(S1)?)?
        .into_iter()
        .rev()
        .find(|row| row.record.hook == HookKind::Prompt)
        .ok_or("no prompt's retrieval recorded")?;
    let mut files = Vec::new();
    for file in &latest.record.files {
        let (_, path) = queries.node_file(file.node)?.ok_or("a ranked file")?;
        files.push(path.as_str().to_owned());
    }
    Ok(files)
}

/// The session's prompt after a failed run seeds from the frames the run went through: with the
/// map on, `calc/ops.py`, where `divide` failed, ranks higher for "The tests fail. Fix it so they
/// pass." than the same prompt ranks it with the map off, everything else alike.
#[test]
fn the_next_prompt_seeds_from_the_failure() -> TestResult {
    let mut places = Vec::new();
    for config in [
        "mode = \"medium\"\n[holdout]\nshare = 0\n[hints]\nruntime_grounding = true\n",
        "mode = \"medium\"\n[holdout]\nshare = 0\n",
    ] {
        let (home, repo) = repository()?;
        let home = home.path();
        let daemon = started(&repo, home, config)?;
        let at = printed_root(&repo)?;
        given(&failed_run(&repo, &at)?, home)?;
        given(
            &prompt(&repo, "The tests fail. Fix it so they pass.")?,
            home,
        )?;
        let session = Identifier::new(S1)?;
        wait_for(|| {
            let reader = Reader::open(&repo.join(".cortex"), 0)?;
            let prompts = reader
                .queries()
                .injections_of_session(&session)?
                .into_iter()
                .filter(|row| row.record.hook == HookKind::Prompt)
                .count();
            Ok(prompts == 2)
        })?;
        let ranked = ranked_by_latest_prompt(&repo)?;
        let place = ranked
            .iter()
            .position(|path| path == "calc/ops.py")
            .ok_or_else(|| format!("calc/ops.py not ranked: {ranked:?}"))?;
        places.push(place);
        drop(daemon);
    }
    let (on, off) = (places.first(), places.get(1));
    assert!(on < off, "{places:?}");
    Ok(())
}
