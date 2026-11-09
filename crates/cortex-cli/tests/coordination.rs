//! Two agents at once in one repository, through the real binary as each runs it. Claude Code
//! reads `main.py` while Codex changes the signature of `add` in `calc/ops.py`, which `main.py`
//! imports (a reference tied by its import path: 0.8). Codex is told at once that a Claude Code
//! session has read the file that depends on its edit; Claude Code is told at its next hook which
//! definition changed, in which file, and how its own file depends on it. Each is told once; a
//! third session holding an unrelated file is told nothing, and nothing is told of it. When both
//! agents edit the same file, each is told to act first. When a session stops before finishing
//! its task, the next session's first prompt that asks to go on is told what it left. Neither
//! agent messages the other and no tool call is held: each hook only adds context.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::calc::{MAIN, OPS, OPS_FIXED, S1, captured, prompt, read_of_tests, repository};
use common::hook_call::hook;
use common::payloads;
use common::{Daemon, TestResult, graph_of, wait_for};
use cortex_core::{Identifier, SessionStatus};
use cortex_store::{Queries, Reader};
use serde_json::Value;

/// The Codex version the payloads were captured from.
const CODEX: &str = "0.165.0";

/// The Codex session, beside Claude Code's [`S1`].
const CODEX_SESSION: &str = "00000000-0000-4000-8000-0000000000c1";

/// A second Claude Code session, holding a file unrelated to the edit.
const ASIDE: &str = "00000000-0000-4000-8000-0000000000a2";

/// A third Claude Code session, holding the file the edit concerns.
const BESIDE: &str = "00000000-0000-4000-8000-0000000000a3";

/// A Claude Code session that read the file the edit concerns, then ended.
const ENDED: &str = "00000000-0000-4000-8000-0000000000e4";

/// What Claude Code is told of Codex's change to `add` while it holds `main.py`.
const TOLD_CLAUDE: &str = "[cortex] Coordination: Codex changed the signature of add in \
                           calc/ops.py, which your file is linked to by code (0.80); you read \
                           main.py.";

/// What Codex's edit of `add` is warned of besides coordination: the calls to it in `main.py`
/// and the tests, both tied through their imports, and the file's tests.
const ADD_WARNINGS: &str = "[cortex] add's signature changed; 2 call sites still use the old \
                            one: main.py:3, tests/test_ops.py:8\n\
                            [cortex] Tests for calc/ops.py: tests/test_ops.py\n";

/// Runs `cortex hook` as `agent` on `payload`, and returns what it gave the agent.
fn given(agent: &str, payload: &Value, home: &Path) -> TestResult<Option<String>> {
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

/// The file at `path`, written with `/`, under `root` in this platform's form.
fn native(root: &Path, path: &str) -> PathBuf {
    path.split('/')
        .fold(root.to_path_buf(), |at, part| at.join(part))
}

/// Claude Code's captured payload `name` about a read, moved to the file at `path` holding `text`
/// (the result's, when it has one), in `session`.
fn read_of(
    root: &Path,
    name: &str,
    session: &str,
    (path, text): (&str, &str),
) -> TestResult<Value> {
    let mut read = captured(name, root)?;
    read["session_id"] = Value::from(session);
    let file = Value::from(native(root, path).to_string_lossy().into_owned());
    read["tool_input"]["file_path"] = file.clone();
    if read.get("tool_response").is_some() {
        let lines = text.lines().count();
        let result = &mut read["tool_response"]["file"];
        result["filePath"] = file;
        result["content"] = Value::from(text);
        result["numLines"] = Value::from(lines);
        result["totalLines"] = Value::from(lines);
    }
    Ok(read)
}

/// Claude Code's captured prompt with `words`, in `session`.
fn prompt_in(root: &Path, session: &str, words: &str) -> TestResult<Value> {
    let mut asked = prompt(root, words)?;
    asked["session_id"] = Value::from(session);
    Ok(asked)
}

/// Codex's captured payload `name`, in the Codex session.
fn codex(root: &Path, name: &str) -> TestResult<Value> {
    let mut payload: Value =
        serde_json::from_slice(&payloads::payload_of("codex", CODEX, name, root)?)?;
    payload["session_id"] = Value::from(CODEX_SESSION);
    Ok(payload)
}

/// Codex's captured patch made one that adds a third number to `add`, after writing the file it
/// leaves: `before` with `add`'s header changed.
fn codex_changes_add(root: &Path, before: &str) -> TestResult<Value> {
    std::fs::write(
        root.join("calc/ops.py"),
        before.replace("def add(a, b):", "def add(a, b, c=0):"),
    )?;
    let mut patch = codex(root, "post-tool-use-apply-patch.json")?;
    patch["tool_input"]["command"] = Value::from(
        "*** Begin Patch\n*** Update File: calc/ops.py\n@@\n-def add(a, b):\n+def add(a, b, c=0):\n     return a + b\n*** End Patch",
    );
    Ok(patch)
}

/// Reads the store of the repository at `root` with `read`.
fn read<T>(root: &Path, read: impl FnOnce(Queries<'_>) -> TestResult<T>) -> TestResult<T> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    read(reader.queries())
}

/// The texts of the notices waiting for `session`.
fn waiting(root: &Path, session: &str) -> TestResult<Vec<String>> {
    let session = Identifier::new(session)?;
    read(root, |queries| {
        Ok(queries
            .notices_waiting(&session, 0)?
            .into_iter()
            .map(|notice| notice.text)
            .collect())
    })
}

/// The texts `session` was sent or given.
fn noticed(root: &Path, session: &str) -> TestResult<Vec<String>> {
    let session = Identifier::new(session)?;
    read(root, |queries| {
        Ok(queries
            .notices_since(&session, 0)?
            .into_iter()
            .map(|notice| notice.text)
            .collect())
    })
}

/// Starts the daemon on the captured repository, pinned to medium mode with `share` of tasks held
/// out, once its graph is built.
fn started(share: u8) -> TestResult<(tempfile::TempDir, PathBuf, Daemon)> {
    let (home, repo) = repository()?;
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(
        repo.join(".cortex/config.toml"),
        format!("mode = \"medium\"\n[holdout]\nshare = {share}\n"),
    )?;
    let daemon = Daemon::start(&repo, home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home.path())?.is_some()))?;
    Ok((home, repo, daemon))
}

/// Codex is told at once that Claude Code read a file its edit concerns; Claude Code at its next
/// hook which definition changed and how its file depends on it; each once, and the session
/// reading an unrelated file not at all.
#[test]
fn two_agents_are_each_told_once_of_the_other() -> TestResult {
    let (home, repo, _daemon) = started(0)?;
    let home = home.path();
    given(
        "claude",
        &prompt_in(&repo, S1, "Print the total in main.py.")?,
        home,
    )?;
    let reads_main = read_of(&repo, "post-tool-use-read.json", S1, ("main.py", MAIN))?;
    assert_eq!(given("claude", &reads_main, home)?, None);
    given(
        "claude",
        &prompt_in(&repo, ASIDE, "Describe the package.")?,
        home,
    )?;
    let reads_init = read_of(
        &repo,
        "post-tool-use-read.json",
        ASIDE,
        ("calc/__init__.py", ""),
    )?;
    assert_eq!(given("claude", &reads_init, home)?, None);
    wait_for(|| read(&repo, |queries| Ok(queries.positions_since(0)?.len() == 2)))?;
    let mut asked = codex(&repo, "user-prompt-submit.json")?;
    asked["prompt"] = Value::from("Let add take a third number.");
    given("codex", &asked, home)?;
    let codex_session = Identifier::new(CODEX_SESSION)?;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(queries.open_task(&codex_session)?.is_some())
        })
    })?;

    // Codex is told first, ahead of its warnings, of Claude Code's read; not of the other
    // session, whose `calc/__init__.py` nothing links to `calc/ops.py`.
    let patch = codex_changes_add(&repo, OPS)?;
    let told_codex = "[cortex] Coordination: Claude Code has read main.py, linked by code (0.80) \
                      to calc/ops.py.\n";
    assert_eq!(
        given("codex", &patch, home)?,
        Some(format!("{told_codex}{ADD_WARNINGS}"))
    );

    // Claude Code is told at its next hook, before a read of the tests; then never again.
    let told_claude = "[cortex] Coordination: Codex changed the signature of add in calc/ops.py, \
                       which your file is linked to by code (0.80); you read main.py.";
    wait_for(|| Ok(!waiting(&repo, S1)?.is_empty()))?;
    let next = read_of(
        &repo,
        "pre-tool-use-read.json",
        S1,
        ("tests/test_ops.py", ""),
    )?;
    assert_eq!(
        given("claude", &next, home)?,
        Some(format!("{told_claude}\n"))
    );
    assert_eq!(given("claude", &next, home)?, None);
    let aside_next = read_of(
        &repo,
        "pre-tool-use-read.json",
        ASIDE,
        ("calc/__init__.py", ""),
    )?;
    assert_eq!(given("claude", &aside_next, home)?, None);
    // The same edit again tells Codex nothing: its line was told, its warnings too.
    assert_eq!(given("codex", &patch, home)?, None);

    // The store keeps each notice once: Claude Code's delivered, Codex's line as told, none for
    // the other session.
    wait_for(|| Ok(waiting(&repo, S1)?.is_empty()))?;
    assert_eq!(noticed(&repo, S1)?, [told_claude]);
    assert_eq!(
        noticed(&repo, CODEX_SESSION)?,
        [told_codex.trim_end_matches('\n')]
    );
    assert_eq!(noticed(&repo, ASIDE)?, Vec::<String>::new());

    // Claude Code edits `main.py`, linked to the `calc/ops.py` Codex is editing. It is not told
    // that Codex edited `calc/ops.py`: it was told so, with the signature changed, in other words.
    // Codex's notice of the edit, which it was never told, is queued.
    std::fs::write(repo.join("main.py"), MAIN_THREE)?;
    let edits_main = edit_of(&repo, "main.py", (MAIN, MAIN_THREE))?;
    let answer = given("claude", &edits_main, home)?.unwrap_or_default();
    assert!(!answer.contains("[cortex] Coordination"), "{answer}");
    wait_for(|| Ok(!waiting(&repo, CODEX_SESSION)?.is_empty()))?;
    assert_eq!(
        waiting(&repo, CODEX_SESSION)?,
        [
            "[cortex] Coordination: Claude Code edited main.py, which your file is linked to by \
             code (0.80); you are editing calc/ops.py."
        ]
    );
    Ok(())
}

/// `main.py` calling `add` with three numbers.
const MAIN_THREE: &str = "from calc.ops import add\n\nprint(add(1, 2, 3))\n";

/// Claude Code's captured edit, moved to `path` in `root`, whose one changed line turns `before`
/// into `after`; both differ in their third line only.
fn edit_of(root: &Path, path: &str, (before, after): (&str, &str)) -> TestResult<Value> {
    let mut edit = captured("post-tool-use-edit.json", root)?;
    let file = Value::from(native(root, path).to_string_lossy().into_owned());
    let line = |text: &str| text.lines().nth(2).unwrap_or_default().to_owned();
    let (old, new) = (line(before), line(after));
    edit["tool_input"]["file_path"] = file.clone();
    edit["tool_input"]["old_string"] = Value::from(old.clone());
    edit["tool_input"]["new_string"] = Value::from(new.clone());
    let result = &mut edit["tool_response"];
    result["filePath"] = file;
    result["oldString"] = Value::from(old.clone());
    result["newString"] = Value::from(new.clone());
    result["originalFile"] = Value::from(before);
    result["structuredPatch"] = serde_json::json!([{
        "oldStart": 3, "oldLines": 1, "newStart": 3, "newLines": 1,
        "lines": [format!("-{old}"), format!("+{new}")],
    }]);
    Ok(edit)
}

/// Claude Code edits `calc/ops.py` and Codex edits it too within minutes: each is told to act
/// first, Codex at once and Claude Code at its next hook.
#[test]
fn two_agents_editing_one_file_are_both_told_to_act_first() -> TestResult {
    let (home, repo, _daemon) = started(0)?;
    let home = home.path();
    given(
        "claude",
        &prompt_in(&repo, S1, "Make divide return true division.")?,
        home,
    )?;
    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    assert_eq!(
        given("claude", &captured("post-tool-use-edit.json", &repo)?, home)?,
        Some("[cortex] Tests for calc/ops.py: tests/test_ops.py\n".to_owned())
    );
    wait_for(|| read(&repo, |queries| Ok(queries.positions_since(0)?.len() == 1)))?;
    let mut asked = codex(&repo, "user-prompt-submit.json")?;
    asked["prompt"] = Value::from("Let add take a third number.");
    given("codex", &asked, home)?;
    let codex_session = Identifier::new(CODEX_SESSION)?;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(queries.open_task(&codex_session)?.is_some())
        })
    })?;

    let patch = codex_changes_add(&repo, OPS_FIXED)?;
    assert_eq!(
        given("codex", &patch, home)?,
        Some(format!(
            "[cortex] Coordination, act first: Claude Code is also editing calc/ops.py, the file \
             you just edited.\n{ADD_WARNINGS}"
        ))
    );
    wait_for(|| Ok(!waiting(&repo, S1)?.is_empty()))?;
    let next = read_of(&repo, "pre-tool-use-read.json", S1, ("main.py", ""))?;
    assert_eq!(
        given("claude", &next, home)?,
        Some(
            "[cortex] Coordination, act first: Codex changed the signature of add in \
             calc/ops.py, the file you are editing.\n"
                .to_owned()
        )
    );
    assert_eq!(given("claude", &next, home)?, None);
    Ok(())
}

/// Every task held out: neither agent is told anything Cortex would say, yet the notice Codex's
/// edit made for Claude Code is queued, and waits.
#[test]
fn a_held_out_task_is_told_nothing_and_its_edit_still_queues() -> TestResult {
    let (home, repo, _daemon) = started(1)?;
    let home = home.path();
    given(
        "claude",
        &prompt_in(&repo, S1, "Print the total in main.py.")?,
        home,
    )?;
    let reads_main = read_of(&repo, "post-tool-use-read.json", S1, ("main.py", MAIN))?;
    given("claude", &reads_main, home)?;
    // Claude Code's answer ends, and its task with it: what its next prompt is given is decided
    // by the task that prompt starts, held out as every task here.
    let mut stop = captured("stop.json", &repo)?;
    stop["session_id"] = Value::from(S1);
    given("claude", &stop, home)?;
    let s1 = Identifier::new(S1)?;
    wait_for(|| read(&repo, |queries| Ok(queries.open_task(&s1)?.is_none())))?;
    wait_for(|| read(&repo, |queries| Ok(queries.positions_since(0)?.len() == 1)))?;
    let mut asked = codex(&repo, "user-prompt-submit.json")?;
    asked["prompt"] = Value::from("Let add take a third number.");
    given("codex", &asked, home)?;
    let codex_session = Identifier::new(CODEX_SESSION)?;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(queries.open_task(&codex_session)?.is_some())
        })
    })?;

    assert_eq!(given("codex", &codex_changes_add(&repo, OPS)?, home)?, None);
    wait_for(|| Ok(!waiting(&repo, S1)?.is_empty()))?;
    let again = prompt_in(&repo, S1, "Now print the difference too.")?;
    assert_eq!(given("claude", &again, home)?, None);
    let next = read_of(
        &repo,
        "pre-tool-use-read.json",
        S1,
        ("tests/test_ops.py", ""),
    )?;
    assert_eq!(given("claude", &next, home)?, None);
    assert_eq!(
        waiting(&repo, S1)?,
        [
            "[cortex] Coordination: Codex changed the signature of add in calc/ops.py, which your \
          file is linked to by code (0.80); you read main.py."
        ]
    );
    assert_eq!(noticed(&repo, CODEX_SESSION)?, Vec::<String>::new());
    Ok(())
}

/// Claude Code edits `calc/ops.py` while three other sessions hold `main.py`, which imports it:
/// a subagent Claude Code started, a Claude Code session that has since ended, and Codex. Claude
/// Code is told of Codex alone, and only Codex is queued a notice: a subagent works with the
/// session that started it, and a session that ended hears nothing more.
#[test]
fn an_edit_tells_neither_its_own_subagents_nor_ended_sessions() -> TestResult {
    let (home, repo, _daemon) = started(0)?;
    let home = home.path();
    given(
        "claude",
        &prompt_in(&repo, S1, "Make divide return true division.")?,
        home,
    )?;
    let mut subagent_reads = read_of(&repo, "post-tool-use-read.json", S1, ("main.py", MAIN))?;
    subagent_reads["agent_id"] = Value::from("explorer1");
    subagent_reads["agent_type"] = Value::from("Explore");
    given("claude", &subagent_reads, home)?;
    given(
        "claude",
        &prompt_in(&repo, ENDED, "Print the total in main.py.")?,
        home,
    )?;
    let ended_reads = read_of(&repo, "post-tool-use-read.json", ENDED, ("main.py", MAIN))?;
    given("claude", &ended_reads, home)?;
    let mut end = captured("session-end.json", &repo)?;
    end["session_id"] = Value::from(ENDED);
    given("claude", &end, home)?;
    let mut asked = codex(&repo, "user-prompt-submit.json")?;
    asked["prompt"] = Value::from("Explain what main.py prints.");
    given("codex", &asked, home)?;
    let mut codex_reads = codex(&repo, "post-tool-use-bash-read.json")?;
    codex_reads["tool_input"]["command"] = Value::from("Get-Content -Raw main.py");
    codex_reads["tool_response"] = Value::from(MAIN);
    given("codex", &codex_reads, home)?;
    let ended = Identifier::new(ENDED)?;
    wait_for(|| {
        read(&repo, |queries| {
            let seen = queries.sessions_seen(std::slice::from_ref(&ended))?;
            let is_ended = seen.iter().any(|session| session.ended_ms.is_some());
            Ok(is_ended && queries.positions_since(0)?.len() == 3)
        })
    })?;

    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    assert_eq!(
        given("claude", &captured("post-tool-use-edit.json", &repo)?, home)?,
        Some(
            "[cortex] Coordination: Codex has read main.py, linked by code (0.80) to \
             calc/ops.py.\n[cortex] Tests for calc/ops.py: tests/test_ops.py\n"
                .to_owned()
        )
    );
    wait_for(|| Ok(!waiting(&repo, CODEX_SESSION)?.is_empty()))?;
    assert_eq!(
        waiting(&repo, CODEX_SESSION)?,
        [
            "[cortex] Coordination: Claude Code edited calc/ops.py, which your file is linked \
             to by code (0.80); you read main.py."
        ]
    );
    let subagent = format!("{S1}/explorer1");
    assert_eq!(waiting(&repo, &subagent)?, Vec::<String>::new());
    assert_eq!(waiting(&repo, ENDED)?, Vec::<String>::new());
    Ok(())
}

/// A notice waits for any hook that can carry text: Claude Code gets it at a prompt that asks for
/// no code ("ok"), and a second Claude Code session holding the same file at a shell command's
/// result; each once. Codex, told of both sessions, is told of them in one line, since the line
/// says the same of each.
#[test]
fn a_notice_reaches_a_short_prompt_and_a_shell_s_result() -> TestResult {
    let (home, repo, _daemon) = started(0)?;
    let home = home.path();
    for session in [S1, BESIDE] {
        given(
            "claude",
            &prompt_in(&repo, session, "Print the total in main.py.")?,
            home,
        )?;
        let reads_main = read_of(&repo, "post-tool-use-read.json", session, ("main.py", MAIN))?;
        given("claude", &reads_main, home)?;
    }
    wait_for(|| read(&repo, |queries| Ok(queries.positions_since(0)?.len() == 2)))?;
    let mut asked = codex(&repo, "user-prompt-submit.json")?;
    asked["prompt"] = Value::from("Let add take a third number.");
    given("codex", &asked, home)?;
    let codex_session = Identifier::new(CODEX_SESSION)?;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(queries.open_task(&codex_session)?.is_some())
        })
    })?;
    assert_eq!(
        given("codex", &codex_changes_add(&repo, OPS)?, home)?,
        Some(format!(
            "[cortex] Coordination: Claude Code has read main.py, linked by code (0.80) to \
             calc/ops.py.\n{ADD_WARNINGS}"
        ))
    );
    wait_for(|| Ok(!waiting(&repo, S1)?.is_empty() && !waiting(&repo, BESIDE)?.is_empty()))?;

    let ok = prompt_in(&repo, S1, "ok")?;
    assert_eq!(
        given("claude", &ok, home)?,
        Some(format!("{TOLD_CLAUDE}\n"))
    );
    assert_eq!(given("claude", &ok, home)?, None);
    let mut shell = captured("post-tool-use-bash.json", &repo)?;
    shell["session_id"] = Value::from(BESIDE);
    assert_eq!(
        given("claude", &shell, home)?,
        Some(format!("{TOLD_CLAUDE}\n"))
    );
    assert_eq!(given("claude", &shell, home)?, None);
    Ok(())
}

/// The open handoffs of the repository at `root`.
fn open_handoffs(root: &Path) -> TestResult<usize> {
    read(root, |queries| Ok(queries.open_handoffs_since(0)?.len()))
}

/// Claude Code reads the tests, edits `calc/ops.py`, and its turn stops on a usage limit before it
/// finishes. A session that was already at work asks to continue and is told nothing. Codex's
/// first prompt asks the same and is told, once, what was left: the agent, when, the file edited
/// and the file read (no check ran, no warning was open). Codex's next prompt is told nothing, and
/// neither is a later session asking the same, the handoff having been taken.
#[test]
fn a_task_left_unfinished_is_handed_to_the_next_session() -> TestResult {
    let (home, repo, _daemon) = started(0)?;
    let home = home.path();
    given(
        "claude",
        &prompt_in(&repo, BESIDE, "Explain what main.py prints.")?,
        home,
    )?;
    given(
        "claude",
        &prompt_in(&repo, S1, "Make divide return true division.")?,
        home,
    )?;
    given("claude", &read_of_tests(&repo)?, home)?;
    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    given("claude", &captured("post-tool-use-edit.json", &repo)?, home)?;
    let mut stopped: Value = serde_json::from_slice(&payloads::payload_of(
        "claude",
        "2.1.318",
        "stop-failure-model-not-found.json",
        &repo,
    )?)?;
    stopped["session_id"] = Value::from(S1);
    stopped["error"] = Value::from("rate_limit");
    assert_eq!(given("claude", &stopped, home)?, None);
    wait_for(|| Ok(open_handoffs(&repo)? == 1))?;
    let beside = Identifier::new(BESIDE)?;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(!queries.tasks_of_session(&beside)?.is_empty())
        })
    })?;
    assert_eq!(
        given("claude", &prompt_in(&repo, BESIDE, "Continue.")?, home)?,
        None
    );
    assert_eq!(open_handoffs(&repo)?, 1);

    let mut asked = codex(&repo, "user-prompt-submit.json")?;
    asked["prompt"] = Value::from("Continue.");
    assert_eq!(
        given("codex", &asked, home)?,
        Some(
            "[cortex] Handoff: a Claude Code session stopped less than a minute ago before \
             finishing its task. It edited calc/ops.py. It had read tests/test_ops.py.\n"
                .to_owned()
        )
    );
    wait_for(|| Ok(open_handoffs(&repo)? == 0))?;
    let codex_session = Identifier::new(CODEX_SESSION)?;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(!queries.tasks_of_session(&codex_session)?.is_empty())
        })
    })?;
    assert_eq!(given("codex", &asked, home)?, None);
    assert_eq!(
        given("claude", &prompt_in(&repo, ASIDE, "Continue.")?, home)?,
        None
    );
    Ok(())
}

/// Claude Code starts a task and its subagent edits `calc/ops.py`, with the parent named by `<session>/<agent>`.
fn subagent_edits_ops(repo: &Path, home: &Path) -> TestResult<Option<String>> {
    given(
        "claude",
        &prompt_in(repo, S1, "Make divide return true division.")?,
        home,
    )?;
    let parent = Identifier::new(S1)?;
    wait_for(|| read(repo, |queries| Ok(queries.open_task(&parent)?.is_some())))?;
    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    let mut edit = captured("post-tool-use-edit.json", repo)?;
    edit["agent_id"] = Value::from("worker1");
    edit["agent_type"] = Value::from("general-purpose");
    given("claude", &edit, home)
}

/// A subagent works in the task of the session that started it: its edit of `calc/ops.py` names
/// the file's tests, as the parent's own edit would.
#[test]
fn a_subagent_s_edit_is_answered_in_its_parent_s_task() -> TestResult {
    let (home, repo, _daemon) = started(0)?;
    assert_eq!(
        subagent_edits_ops(&repo, home.path())?,
        Some("[cortex] Tests for calc/ops.py: tests/test_ops.py\n".to_owned())
    );
    Ok(())
}

/// Every task held out: the subagent's edit is held out with its parent's task and told nothing.
#[test]
fn a_subagent_is_held_out_with_its_parent() -> TestResult {
    let (home, repo, _daemon) = started(1)?;
    assert_eq!(subagent_edits_ops(&repo, home.path())?, None);
    Ok(())
}

/// Claude Code edits `calc/ops.py` and its answer ends with no check run: the task leaves a
/// handoff, but its session is still at work, so Codex asking to continue is told nothing. Once
/// the session ends, a new session asking the same is told the session ended without a passing
/// check, what it edited, and that the warning its edit got (the file's tests, never run) is still
/// open.
#[test]
fn a_handoff_waits_until_its_session_is_done() -> TestResult {
    let (home, repo, _daemon) = started(0)?;
    let home = home.path();
    given(
        "claude",
        &prompt_in(&repo, S1, "Make divide return true division.")?,
        home,
    )?;
    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    given("claude", &captured("post-tool-use-edit.json", &repo)?, home)?;
    given("claude", &captured("stop.json", &repo)?, home)?;
    wait_for(|| Ok(open_handoffs(&repo)? == 1))?;

    let mut asked = codex(&repo, "user-prompt-submit.json")?;
    asked["prompt"] = Value::from("Continue.");
    assert_eq!(given("codex", &asked, home)?, None);

    given("claude", &captured("session-end.json", &repo)?, home)?;
    let ended = Identifier::new(S1)?;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(queries
                .session(&ended)?
                .is_some_and(|row| row.ended_at_ms.is_some()))
        })
    })?;
    assert_eq!(
        given("claude", &prompt_in(&repo, ASIDE, "Continue.")?, home)?,
        Some(
            "[cortex] Handoff: a Claude Code session answered less than a minute ago without a \
             passing check. It edited calc/ops.py. 1 warning about its edits was still open.\n"
                .to_owned()
        )
    );
    Ok(())
}

/// A session its user cleared (`/clear`, a session end with reason `clear`) set its work aside on
/// purpose: what it left is handed to nobody, not even the session `/clear` starts.
#[test]
fn a_cleared_session_hands_nothing_on() -> TestResult {
    let (home, repo, _daemon) = started(0)?;
    let home = home.path();
    given(
        "claude",
        &prompt_in(&repo, S1, "Make divide return true division.")?,
        home,
    )?;
    std::fs::write(repo.join("calc/ops.py"), OPS_FIXED)?;
    given("claude", &captured("post-tool-use-edit.json", &repo)?, home)?;
    given("claude", &captured("stop.json", &repo)?, home)?;
    wait_for(|| Ok(open_handoffs(&repo)? == 1))?;
    let mut cleared = captured("session-end.json", &repo)?;
    cleared["reason"] = Value::from("clear");
    given("claude", &cleared, home)?;
    let ended = Identifier::new(S1)?;
    wait_for(|| {
        read(&repo, |queries| {
            Ok(queries
                .session(&ended)?
                .is_some_and(|row| row.status == SessionStatus::Cleared))
        })
    })?;
    assert_eq!(
        given("claude", &prompt_in(&repo, ASIDE, "Continue.")?, home)?,
        None
    );
    let mut asked = codex(&repo, "user-prompt-submit.json")?;
    asked["prompt"] = Value::from("Continue.");
    assert_eq!(given("codex", &asked, home)?, None);
    Ok(())
}
