//! Why-lines through the real binary as Claude Code runs it, with `history.why` on, in the
//! calculator repository with a scripted history: a fix of `add` that names an issue, gives a
//! reason and touches the tests, then a formatting commit over its line. A session asks about
//! `add`: the first work order has no why-line, since nothing is blamed
//! yet, and asks the daemon's history worker for the blame; once the blame is kept, the same
//! prompt's work order carries the fix's reason under the code, the formatting commit looked
//! through.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use common::calc::{MAIN, TESTS, captured, prompt, repository};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, isolate, wait_for};
use cortex_core::ContentHash;
use cortex_store::Reader;
use serde_json::Value;

/// `calc/ops.py` as HEAD holds it: `add`, on lines 4 and 5, adds, the sum in parentheses since the
/// formatting commit.
const OPS_HEAD: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return (a + b)\n\n\n\
                        def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a / b\n";

/// `calc/ops.py` before the fix: `add` subtracts.
const OPS_BUG: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a - b\n\n\n\
                       def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a / b\n";

/// `calc/ops.py` after the fix, before the formatting commit.
const OPS_FIXED: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a + b\n\n\n\
                         def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a / b\n";

/// The tests before the fix: none of `add`.
const TESTS_BEFORE: &str = "import unittest\n\nfrom calc.ops import divide, mul\n\n\n\
                            class OpsTest(unittest.TestCase):\n    def test_mul(self):\n        \
                            self.assertEqual(mul(2, 3), 6)\n";

/// 2026-01-01T00:00:00Z, in seconds: day 0 of the history.
const DAY_0_S: i64 = 1_767_225_600;

/// A second session, which asks the same once the blame is kept.
const LATER: &str = "00000000-0000-4000-8000-000000000002";

/// The prompt: it names `add` and the file it is in.
const ASKED: &str = "Make add in calc/ops.py check that both arguments are numbers.";

/// The why-line the fix gives line 5, typed by hand: its subject, its reason line, the issue it
/// fixes and the test file it touched, and the day it was committed (day 2, 2026-01-03).
const WHY_5: &str = "  why 5: commit \"Make add return the sum\": \"Addition subtracted its second argument.\" (fixes #7; test: tests/test_ops.py), 2026-01-03";

/// Commits every change in the repository at `root` on day `day` as `message`, with `home` as the
/// user's home folder: the new commit's id.
fn commit(root: &Path, home: &Path, day: i64, message: &str) -> TestResult<String> {
    let time = format!("{} +0000", DAY_0_S + day * 86_400);
    for args in [&["add", "-A"][..], &["commit", "-q", "-m", message][..]] {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(root)
            .env("GIT_AUTHOR_NAME", "Cortex Test")
            .env("GIT_AUTHOR_EMAIL", "test")
            .env("GIT_COMMITTER_NAME", "Cortex Test")
            .env("GIT_COMMITTER_EMAIL", "test")
            .env("GIT_AUTHOR_DATE", &time)
            .env("GIT_COMMITTER_DATE", &time);
        isolate(&mut command, home);
        if !command.status()?.success() {
            return Err(format!("git {args:?} failed").into());
        }
    }
    let mut head = Command::new("git");
    head.args(["rev-parse", "HEAD"]).current_dir(root);
    isolate(&mut head, home);
    Ok(String::from_utf8(head.output()?.stdout)?.trim().to_owned())
}

/// Runs git with `args` in `root`, with `home` as the user's home folder.
fn git(root: &Path, home: &Path, args: &[&str]) -> TestResult {
    let mut command = Command::new("git");
    command.args(args).current_dir(root);
    isolate(&mut command, home);
    if !command.status()?.success() {
        return Err(format!("git {args:?} failed").into());
    }
    Ok(())
}

/// The calculator's history: the code on day 0, its tests on day 1, the fix of `add` with its test
/// on day 2, and the formatting commit on day 3: the fix's id.
fn history(root: &Path, home: &Path) -> TestResult<String> {
    git(root, home, &["init", "-q"])?;
    git(root, home, &["config", "core.autocrlf", "false"])?;
    std::fs::write(root.join(".git/info/exclude"), ".cortex/\n")?;
    std::fs::write(root.join("calc/ops.py"), OPS_BUG)?;
    std::fs::write(root.join("main.py"), MAIN)?;
    std::fs::remove_dir_all(root.join("tests"))?;
    commit(root, home, 0, "add the calculator")?;
    std::fs::create_dir_all(root.join("tests"))?;
    std::fs::write(root.join("tests/__init__.py"), "")?;
    std::fs::write(root.join("tests/test_ops.py"), TESTS_BEFORE)?;
    commit(root, home, 1, "add the tests")?;
    std::fs::write(root.join("calc/ops.py"), OPS_FIXED)?;
    std::fs::write(root.join("tests/test_ops.py"), TESTS)?;
    let fix = commit(
        root,
        home,
        2,
        "Make add return the sum\n\nAddition subtracted its second argument.\n\nFixes #7",
    )?;
    std::fs::write(root.join("calc/ops.py"), OPS_HEAD)?;
    commit(root, home, 3, "style: parenthesise the sum")?;
    Ok(fix)
}

/// The daemon on the repository at `root`, with `home` as the user's home folder, in medium mode
/// with `history.why` on and nothing held out, once its graph is built.
fn started(root: &Path, home: &Path) -> TestResult<Daemon> {
    std::fs::create_dir_all(root.join(".cortex"))?;
    std::fs::write(
        root.join(".cortex/config.toml"),
        "mode = \"medium\"\nhistory.why = true\n[holdout]\nshare = 0\n",
    )?;
    let daemon = Daemon::start(root, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(root, home)?.is_some()))?;
    Ok(daemon)
}

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

/// The hash the store of the repository at `root` holds for `calc/ops.py`, as the index read it.
fn indexed_hash(root: &Path) -> TestResult<Option<ContentHash>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let ops = reader
        .queries()
        .file(&cortex_core::RepoPath::new("calc/ops.py")?)?;
    Ok(ops.and_then(|ops| ops.file.content_hash))
}

/// Whether the store of the repository at `root` keeps a blame naming the commit `oid`.
fn has_blame_naming(root: &Path, oid: &str) -> TestResult<bool> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let ops = reader
        .queries()
        .file(&cortex_core::RepoPath::new("calc/ops.py")?)?
        .ok_or("calc/ops.py is not indexed")?;
    let hash = ops.file.content_hash.ok_or("no content hash")?;
    Ok(reader
        .queries()
        .cached_blames((&ops.file.path, &hash), 1, 13)?
        .iter()
        .any(|blame| blame.runs.iter().any(|run| run.oid == oid)))
}

/// A session starts and asks about `add`. Its work order shows `add`'s code and, the cache
/// cold, no why-line; the history worker is asked for the blame,
/// and once the store keeps it, naming the fix, the same prompt from a new session gets a work
/// order that carries the fix's
/// why-line for line 5, the formatting commit that owns that line at HEAD looked through.
#[test]
fn a_work_order_carries_the_reason_once_the_blame_is_kept() -> TestResult {
    let (home, repo) = repository()?;
    let fix = history(&repo, home.path())?;
    let _daemon = started(&repo, home.path())?;
    let home = home.path();

    given(&captured("session-start-startup.json", &repo)?, home)?;
    let asked = prompt(&repo, ASKED)?;
    let cold = given(&asked, home)?.ok_or("no work order")?;
    assert!(cold.contains("  calc/ops.py:"), "{cold}");
    assert!(!cold.contains("  why "), "the premise: {cold}");

    wait_for(|| has_blame_naming(&repo, &fix))?;
    // From a session of its own: the first session was given the span already, and is shown the
    // next one.
    let mut start = captured("session-start-startup.json", &repo)?;
    start["session_id"] = Value::from(LATER);
    given(&start, home)?;
    let mut again = asked.clone();
    again["session_id"] = Value::from(LATER);
    let warm = given(&again, home)?.ok_or("no work order")?;
    assert!(
        warm.lines().any(|line| line == WHY_5),
        "{WHY_5}\nin\n{warm}"
    );
    let without: Vec<&str> = warm
        .lines()
        .filter(|line| !line.starts_with("  why "))
        .collect();
    let cold_lines: Vec<&str> = cold.lines().collect();
    assert_eq!(without, cold_lines);
    Ok(())
}

/// `calc/ops.py` once the agent's edit made `add` convert its arguments, on line 5.
const OPS_EDITED: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    \
                          return float(a) + float(b)\n\n\ndef mul(a, b):\n    return a * b\n\n\n\
                          def divide(a, b):\n    return a / b\n";

/// The origin note the edit of line 5 gets, typed by hand: the fix wrote the line, names issue #7
/// and touched the tests, on day 2.
const ORIGIN_5: &str = "[cortex] The line this edit replaced in calc/ops.py came from commit \
                        \"Make add return the sum\" (fixes #7; test: tests/test_ops.py), 2026-01-03.";

/// The captured edit made the edit of line 5 of `calc/ops.py` as HEAD holds it, from session
/// [`LATER`]: one hunk, starting at line 2 with three lines around the change.
fn edit_of_line_5(root: &Path) -> TestResult<Value> {
    let mut edit = captured("post-tool-use-edit.json", root)?;
    edit["session_id"] = Value::from(LATER);
    let (old, new) = ("    return (a + b)", "    return float(a) + float(b)");
    edit["tool_input"]["old_string"] = Value::from(old);
    edit["tool_input"]["new_string"] = Value::from(new);
    let response = &mut edit["tool_response"];
    response["oldString"] = Value::from(old);
    response["newString"] = Value::from(new);
    response["originalFile"] = Value::from(OPS_HEAD);
    let hunk = &mut response["structuredPatch"][0];
    for field in ["oldStart", "newStart"] {
        hunk[field] = Value::from(2);
    }
    for field in ["oldLines", "newLines"] {
        hunk[field] = Value::from(7);
    }
    hunk["lines"] = Value::from(vec![
        " ".to_owned(),
        " ".to_owned(),
        " def add(a, b):".to_owned(),
        format!("-{old}"),
        format!("+{new}"),
        " ".to_owned(),
        " ".to_owned(),
        " def mul(a, b):".to_owned(),
    ]);
    Ok(edit)
}

/// Once the blame of `calc/ops.py` is kept, a session asks about `add` and edits its line 5, which
/// the fix wrote (the formatting commit that owns it at HEAD looked through): the edit is told the
/// line came from the fix, with the issue and the test it names. The edit's event is sent only once
/// the watcher has indexed the edited file, so the note comes from the blame of the file before
/// the edit, as the agent reported it, and not from the index having fallen behind.
#[test]
fn an_edit_is_told_where_the_lines_it_replaced_came_from() -> TestResult {
    let (home, repo) = repository()?;
    let fix = history(&repo, home.path())?;
    let _daemon = started(&repo, home.path())?;
    let home = home.path();
    given(&captured("session-start-startup.json", &repo)?, home)?;
    let asked = prompt(&repo, ASKED)?;
    given(&asked, home)?;
    wait_for(|| has_blame_naming(&repo, &fix))?;

    let mut start = captured("session-start-startup.json", &repo)?;
    start["session_id"] = Value::from(LATER);
    given(&start, home)?;
    let mut again = asked;
    again["session_id"] = Value::from(LATER);
    let warm = given(&again, home)?.ok_or("no work order")?;
    assert!(
        warm.lines().any(|line| line == WHY_5),
        "the premise: {warm}"
    );

    std::fs::write(repo.join("calc/ops.py"), OPS_EDITED)?;
    let edited = ContentHash::of(OPS_EDITED.as_bytes());
    wait_for(|| Ok(indexed_hash(&repo)? == Some(edited)))?;
    let told = given(&edit_of_line_5(&repo)?, home)?.ok_or("nothing told")?;
    assert!(
        told.lines().any(|line| line == ORIGIN_5),
        "{ORIGIN_5}\nin\n{told}"
    );
    Ok(())
}
