//! The note a check gets before it runs again, through the real binary as Claude Code and Codex run
//! it, on the repository the captured payloads come from. Each run is sent as the agent sends it:
//! the moment before it runs, then its result. Once the captured unittest run failed, the moment
//! before the same command runs in the same folder is told what that run gave, the failing test
//! its output named, and the note is recorded as given; a third run of it in the task is told
//! nothing. What silences it: an edit a hook reports, a file the watcher sees change, a shell
//! command that may have written a file (not one that only reads), an edit while the run was going,
//! a result whose start was never heard, a line that writes besides its check, and a failed result
//! that printed nothing. Another command or folder, a session with no open task, the key left at
//! its default, a held-out task and a tiny repository are told nothing. The daemon being down is
//! covered where every hook is: `daemon_down.rs` sends the moment before a shell command too.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::time::Duration;

use common::calc::{S1, captured, prompt, repository, tests_run};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, payloads, wait_for};
use cortex_core::{HookKind, Identifier, RepoPath};
use cortex_store::Reader;
use serde_json::Value;

/// The line the captured failed run ran.
const FAILED_LINE: &str = "python -m unittest tests.test_fail -q";

/// The note the moment before [`FAILED_LINE`] runs again gets, typed out from the captured run's
/// output: unittest's `FAIL:` header names the one failing test, and Claude Code's `Exit code 1`
/// line its exit code.
const NOTE: &str = "[cortex] `python -m unittest tests.test_fail -q` ran earlier in this session with \
                    no source change since: exit 1, failing: test_known_failure \
                    (tests.test_fail.FailTest.test_known_failure).\n";

/// The configuration with the note on, every task given what Cortex says, in medium mode.
const ON: &str = "mode = \"medium\"\n[holdout]\nshare = 0\n[hints]\nredundant_commands = true\n";

/// Runs `cortex hook` for `agent` on `payload` as the agent does, and returns what it gave.
fn given_to(agent: &str, payload: &Value, home: &Path) -> TestResult<Option<String>> {
    let event = payload["hook_event_name"]
        .as_str()
        .ok_or("no event name")?
        .to_owned();
    let call = hook(
        &["--agent", agent, "--event", &event],
        serde_json::to_vec(payload)?,
        Duration::ZERO,
        home,
    )?;
    call.context(&event)
}

/// Runs `cortex hook` on `payload` as Claude Code does, and returns what it gave the agent.
fn given(payload: &Value, home: &Path) -> TestResult<Option<String>> {
    given_to("claude", payload, home)
}

/// The captured failed unittest run of `line`, as the captured session's: exit code 1, one
/// failing test.
fn failed_run(root: &Path, line: &str) -> TestResult<Value> {
    let mut run = tests_run(root, "post-tool-use-failure-bash-test.json")?;
    run["tool_input"]["command"] = Value::from(line);
    Ok(run)
}

/// The captured moment before a shell command runs, made the moment before `line` runs in `cwd`.
fn before(root: &Path, line: &str, cwd: &Path) -> TestResult<Value> {
    let mut payload = captured("pre-tool-use-bash.json", root)?;
    payload["tool_input"]["command"] = Value::from(line);
    payload["cwd"] = Value::from(cwd.to_string_lossy().into_owned());
    Ok(payload)
}

/// The captured successful shell command, made a run of `line`.
fn shell_done(root: &Path, line: &str) -> TestResult<Value> {
    let mut payload = captured("post-tool-use-bash.json", root)?;
    payload["tool_input"]["command"] = Value::from(line);
    Ok(payload)
}

/// Starts the daemon on `repo` with `config` and, unless `with_task` is false, opens the captured
/// session's task with a prompt.
fn started(repo: &Path, home: &Path, (config, with_task): (&str, bool)) -> TestResult<Daemon> {
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), config)?;
    let daemon = Daemon::start(repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(repo, home)?.is_some()))?;
    if with_task {
        given(&prompt(repo, "Make divide return a true quotient.")?, home)?;
        let session = Identifier::new(S1)?;
        wait_for(|| {
            let reader = Reader::open(&repo.join(".cortex"), 0)?;
            Ok(reader.queries().open_task(&session)?.is_some())
        })?;
    }
    Ok(daemon)
}

/// Sends a run of `line` that failed as the captured one did: the moment before it, then its
/// result; returns what the moment before it was told.
fn run_failed(repo: &Path, home: &Path, line: &str) -> TestResult<Option<String>> {
    let told = given(&before(repo, line, repo)?, home)?;
    assert_eq!(given(&failed_run(repo, line)?, home)?, None);
    Ok(told)
}

/// Starts the daemon with `config`, opens the task, and runs [`FAILED_LINE`] once, failing.
fn failed_once(repo: &Path, home: &Path, config: &str) -> TestResult<Daemon> {
    let daemon = started(repo, home, (config, true))?;
    assert_eq!(run_failed(repo, home, FAILED_LINE)?, None);
    Ok(daemon)
}

/// With the note on, the moment before the failed line runs again in the same folder is told what
/// the run gave, and the note is recorded as given to the task, its 48 tokens counted; another
/// line, or the same line in another folder, is told nothing. Run again and again with nothing
/// changed, the line is told twice in the task, and the third time nothing.
#[test]
fn a_check_run_again_is_told_what_it_gave() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = failed_once(&repo, home, ON)?;
    assert_eq!(
        given(&before(&repo, "python -m unittest -q", &repo)?, home)?,
        None
    );
    assert_eq!(
        given(&before(&repo, FAILED_LINE, &repo.join("calc"))?, home)?,
        None
    );
    assert_eq!(run_failed(&repo, home, FAILED_LINE)?.as_deref(), Some(NOTE));
    let session = Identifier::new(S1)?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        let noted = reader
            .queries()
            .injections_of_session(&session)?
            .into_iter()
            .filter(|row| row.record.hook == HookKind::PreTool)
            .map(|row| (row.record.tokens, row.record.task_id.is_some()))
            .collect::<Vec<_>>();
        Ok(noted == [(48, true)])
    })?;
    assert_eq!(run_failed(&repo, home, FAILED_LINE)?.as_deref(), Some(NOTE));
    assert_eq!(run_failed(&repo, home, FAILED_LINE)?, None);
    drop(daemon);
    Ok(())
}

/// An edit a hook reports after the run silences the note, though the file on disk is left as it
/// was: the daemon heard of a change.
#[test]
fn an_edit_since_the_run_silences_the_note() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = failed_once(&repo, home, ON)?;
    assert_eq!(
        given(&before(&repo, FAILED_LINE, &repo)?, home)?.as_deref(),
        Some(NOTE)
    );
    given(&captured("post-tool-use-edit.json", &repo)?, home)?;
    assert_eq!(given(&before(&repo, FAILED_LINE, &repo)?, home)?, None);
    drop(daemon);
    Ok(())
}

/// A file written after the run with no hook reporting it, once the watcher saw it (the store
/// indexed it), silences the note.
#[test]
fn a_change_the_watcher_saw_silences_the_note() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = failed_once(&repo, home, ON)?;
    assert_eq!(
        given(&before(&repo, FAILED_LINE, &repo)?, home)?.as_deref(),
        Some(NOTE)
    );
    std::fs::write(repo.join("calc/extra.py"), "def extra():\n    return 1\n")?;
    let extra = RepoPath::new("calc/extra.py")?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        Ok(reader.queries().file_node_at(&extra)?.is_some())
    })?;
    assert_eq!(given(&before(&repo, FAILED_LINE, &repo)?, home)?, None);
    drop(daemon);
    Ok(())
}

/// A shell command that only reads, run after the check, leaves the note; one that may have
/// written a file (`sed -i`), whose change the watcher may not have reported yet, silences it.
#[test]
fn a_shell_command_that_may_write_silences_the_note() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = failed_once(&repo, home, ON)?;
    given(&shell_done(&repo, "cat calc/ops.py | head -5")?, home)?;
    assert_eq!(
        given(&before(&repo, FAILED_LINE, &repo)?, home)?.as_deref(),
        Some(NOTE)
    );
    given(&shell_done(&repo, "sed -i s/a/b/ notes.txt")?, home)?;
    assert_eq!(given(&before(&repo, FAILED_LINE, &repo)?, home)?, None);
    drop(daemon);
    Ok(())
}

/// A run during which an edit was reported (after its start, before its result) is not kept: the
/// file may have changed after the run read it. Nor is a run whose start was never heard.
#[test]
fn a_run_that_changed_or_was_not_seen_starting_is_not_kept() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = started(&repo, home, (ON, true))?;
    assert_eq!(given(&before(&repo, FAILED_LINE, &repo)?, home)?, None);
    given(&captured("post-tool-use-edit.json", &repo)?, home)?;
    assert_eq!(given(&failed_run(&repo, FAILED_LINE)?, home)?, None);
    assert_eq!(given(&before(&repo, FAILED_LINE, &repo)?, home)?, None);
    let unseen = "python -m unittest tests.test_fail";
    assert_eq!(given(&failed_run(&repo, unseen)?, home)?, None);
    assert_eq!(given(&before(&repo, unseen, &repo)?, home)?, None);
    drop(daemon);
    Ok(())
}

/// A line that runs the check after a command that writes (`sed -i`) or restores files from git
/// counts as a change of its own, and is not kept.
#[test]
fn a_line_that_writes_besides_its_check_is_not_kept() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = started(&repo, home, (ON, true))?;
    for line in [
        "sed -i s/a/b/ notes.txt && python -m unittest tests.test_fail -q",
        "git checkout -- calc/ops.py && python -m unittest tests.test_fail -q",
    ] {
        assert_eq!(run_failed(&repo, home, line)?, None, "{line}");
        assert_eq!(given(&before(&repo, line, &repo)?, home)?, None, "{line}");
    }
    drop(daemon);
    Ok(())
}

/// A failed run of the line that printed nothing, which the hook sends as the event alone, drops
/// the line's earlier run: what the line gives now is not the note's.
#[test]
fn a_failure_that_printed_nothing_drops_the_run() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = failed_once(&repo, home, ON)?;
    assert_eq!(
        given(&before(&repo, FAILED_LINE, &repo)?, home)?.as_deref(),
        Some(NOTE)
    );
    let mut silent = tests_run(&repo, "post-tool-use-failure-bash.json")?;
    silent["tool_input"]["command"] = Value::from(FAILED_LINE);
    silent["error"] = Value::from("Exit code 1");
    assert_eq!(given(&silent, home)?, None);
    assert_eq!(given(&before(&repo, FAILED_LINE, &repo)?, home)?, None);
    drop(daemon);
    Ok(())
}

/// A session with no open task, which no prompt started, is told nothing.
#[test]
fn a_session_outside_a_task_gets_no_note() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = started(&repo, home, (ON, false))?;
    assert_eq!(run_failed(&repo, home, FAILED_LINE)?, None);
    assert_eq!(given(&before(&repo, FAILED_LINE, &repo)?, home)?, None);
    drop(daemon);
    Ok(())
}

/// With `hints.redundant_commands` left at its default, in a task held out from Cortex (every
/// task is when the share is one) and in a tiny repository, the line run again is told nothing.
#[test]
fn off_held_out_or_tiny_gets_no_note() -> TestResult {
    for config in [
        "mode = \"medium\"\n[holdout]\nshare = 0\n",
        "mode = \"medium\"\n[holdout]\nshare = 1\n[hints]\nredundant_commands = true\n",
        "mode = \"tiny\"\n[holdout]\nshare = 0\n[hints]\nredundant_commands = true\n",
    ] {
        let (home, repo) = repository()?;
        let home = home.path();
        let daemon = failed_once(&repo, home, config)?;
        assert_eq!(
            given(&before(&repo, FAILED_LINE, &repo)?, home)?,
            None,
            "{config}"
        );
        drop(daemon);
    }
    Ok(())
}

/// The Codex version whose captured payloads the Codex test sends.
const CODEX: &str = "0.165.0";

/// The captured Codex payload `name`, its paths moved under `root`, its transcript `rollout`.
fn codex_payload(name: &str, root: &Path, rollout: &Path) -> TestResult<Value> {
    let mut payload: Value =
        serde_json::from_slice(&payloads::payload_of("codex", CODEX, name, root)?)?;
    payload["transcript_path"] = Value::from(rollout.to_string_lossy().into_owned());
    Ok(payload)
}

/// Codex gets the note too: its captured unittest run (`python -m unittest -q`), whose rollout
/// line says it exited 1, is told before it runs again what that run gave, the failing test its
/// output named.
#[test]
fn codex_is_told_too() -> TestResult {
    let (home_folder, repo) = repository()?;
    let home = home_folder.path();
    let daemon = started(&repo, home, (ON, false))?;
    let day = home.join(".codex/sessions/2026/10/29");
    std::fs::create_dir_all(&day)?;
    let rollout = day.join(format!("rollout-2026-10-29T06-03-56-{S1}.jsonl"));
    let result = codex_payload("post-tool-use-bash-test-failing.json", &repo, &rollout)?;
    let id = result["tool_use_id"].as_str().ok_or("no id")?;
    std::fs::write(
        &rollout,
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"item_completed","item":{{"type":"CommandExecution","id":"{id}","status":"completed","exit_code":1}}}}}}"#
        ) + "\n",
    )?;
    given_to(
        "codex",
        &codex_payload("user-prompt-submit.json", &repo, &rollout)?,
        home,
    )?;
    let session = Identifier::new(S1)?;
    wait_for(|| {
        let reader = Reader::open(&repo.join(".cortex"), 0)?;
        Ok(reader.queries().open_task(&session)?.is_some())
    })?;
    let before = codex_payload("pre-tool-use-bash-test.json", &repo, &rollout)?;
    assert_eq!(given_to("codex", &before, home)?, None);
    assert_eq!(given_to("codex", &result, home)?, None);
    assert_eq!(
        given_to("codex", &before, home)?.as_deref(),
        Some(
            "[cortex] `python -m unittest -q` ran earlier in this session with no source change \
             since: exit 1, failing: test_divide (tests.test_ops.OpsTest.test_divide).\n"
        )
    );
    drop(daemon);
    Ok(())
}
