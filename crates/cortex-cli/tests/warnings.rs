//! An edit's warnings, through the real binary as Claude Code runs it, on the repository the
//! captured payloads come from (pinned to medium mode, where every rule runs): an edit that changes
//! `add`'s signature is told of the calls to it in other files and of the file's tests; one that
//! changes `mul`'s once the daemon has indexed it is told of its call, once, though two hooks report
//! it at the same moment; the same edit again is told nothing; and the store keeps one row for each
//! file each warning named. An edit outside a task, of a file the store does not hold, or of a file
//! whose name says it holds secrets is told nothing.
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
use cortex_core::{ContentHash, Identifier, RepoPath, WarningKind};
use cortex_store::{Queries, Reader};
use serde_json::Value;

/// `calc/ops.py` once `add` takes a third number: its header, on line 4, changed.
fn ops_with_add() -> String {
    OPS.replace("def add(a, b):", "def add(a, b, c=0):")
}

/// `calc/ops.py` once `mul` takes a third number as well: its header, on line 8, changed.
fn ops_with_mul() -> String {
    ops_with_add().replace("def mul(a, b):", "def mul(a, b, c=1):")
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

/// The captured edit made an edit of `calc/ops.py` that turned `old` (on line `line`, from the file
/// `before`) into `new`.
fn edit(root: &Path, (old, new): (&str, &str), (line, before): (u64, &str)) -> TestResult<Value> {
    let mut edit = captured("post-tool-use-edit.json", root)?;
    edit["tool_input"]["old_string"] = Value::from(old);
    edit["tool_input"]["new_string"] = Value::from(new);
    let response = &mut edit["tool_response"];
    response["oldString"] = Value::from(old);
    response["newString"] = Value::from(new);
    response["originalFile"] = Value::from(before);
    response["structuredPatch"] = serde_json::json!([{
        "oldStart": line,
        "oldLines": 1,
        "newStart": line,
        "newLines": 1,
        "lines": [format!("-{old}"), format!("+{new}")],
    }]);
    Ok(edit)
}

/// Reads the store of the repository at `root` with `read`.
fn read<T>(root: &Path, read: impl FnOnce(Queries<'_>) -> TestResult<T>) -> TestResult<T> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    read(reader.queries())
}

/// Whether the store holds `calc/ops.py` as `text`: the daemon indexed that change.
fn holds_ops(root: &Path, text: &str) -> TestResult<bool> {
    read(root, |queries| {
        let row = queries.file(&RepoPath::new("calc/ops.py")?)?;
        Ok(row.is_some_and(|row| row.file.content_hash == Some(ContentHash::of(text.as_bytes()))))
    })
}

/// The warnings the store keeps of the captured session's one task, as the kind and the path of
/// the file each named, in the order recorded.
fn warned(root: &Path) -> TestResult<Vec<(WarningKind, String)>> {
    read(root, |queries| {
        let session = Identifier::new(S1)?;
        let Some(task) = queries.tasks_of_session(&session)?.first().cloned() else {
            return Ok(Vec::new());
        };
        let mut warned = Vec::new();
        for row in queries.warnings_of_task(task.record.task_id)? {
            let (_, path) = queries
                .node_file(row.record.target)?
                .ok_or("a named file")?;
            warned.push((row.record.kind, path.as_str().to_owned()));
        }
        Ok(warned)
    })
}

/// An edit that changes a signature is told of the calls to it and of the file's tests, each once a
/// task, whether the daemon indexed the edit before its hook came or not.
#[test]
fn an_edit_is_warned_of_its_callers_and_its_tests() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), "mode = \"medium\"\n")?;
    let daemon = Daemon::start(&repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home)?.is_some()))?;
    given(
        &prompt(&repo, "Let add and mul take a third number.")?,
        home,
    )?;
    let session = Identifier::new(S1)?;
    wait_for(|| read(&repo, |queries| Ok(queries.open_task(&session)?.is_some())))?;

    // `add` is called on line 3 of `main.py` and on line 8 of the tests, both tied through their
    // imports (0.8); the tests import the file (0.8): the call sites come first, by rule order.
    let with_add = ops_with_add();
    std::fs::write(repo.join("calc/ops.py"), &with_add)?;
    let add = edit(&repo, ("def add(a, b):", "def add(a, b, c=0):"), (4, OPS))?;
    assert_eq!(
        given(&add, home)?.as_deref(),
        Some(
            "[cortex] add's signature changed; 2 call sites still use the old one: main.py:3, \
             tests/test_ops.py:8\n\
             [cortex] Tests for calc/ops.py: tests/test_ops.py\n"
        )
    );

    // Once the daemon holds the file as it is, the signature change its index found tells: `mul`
    // is called on line 11 of the tests, which the task was warned of already. Two hooks report the
    // edit at once: one of them tells it.
    let with_mul = ops_with_mul();
    std::fs::write(repo.join("calc/ops.py"), &with_mul)?;
    wait_for(|| holds_ops(&repo, &with_mul))?;
    let mul = edit(
        &repo,
        ("def mul(a, b):", "def mul(a, b, c=1):"),
        (8, &with_add),
    )?;
    let both = std::thread::scope(|scope| {
        let first = scope.spawn(|| given(&mul, home).map_err(|error| error.to_string()));
        let second = scope.spawn(|| given(&mul, home).map_err(|error| error.to_string()));
        [first.join(), second.join()]
    });
    let mut told: Vec<Option<String>> = Vec::new();
    for answer in both {
        told.push(answer.map_err(|_| "a hook's thread panicked")??);
    }
    told.sort();
    assert_eq!(
        told,
        [
            None,
            Some(
                "[cortex] mul's signature changed; 1 call site still uses the old one: \
                 tests/test_ops.py:11\n"
                    .to_owned()
            )
        ]
    );

    // The same edit again: every pair was told in this task.
    assert_eq!(given(&mul, home)?, None);
    let recorded = vec![
        (WarningKind::CallSite, "main.py".to_owned()),
        (WarningKind::CallSite, "tests/test_ops.py".to_owned()),
        (WarningKind::Test, "tests/test_ops.py".to_owned()),
        (WarningKind::CallSite, "tests/test_ops.py".to_owned()),
    ];
    wait_for(|| Ok(warned(&repo)?.len() == recorded.len()))?;
    assert_eq!(warned(&repo)?, recorded);

    // Outside a task: another session, which sent no prompt, edits the same file.
    let mut elsewhere = add.clone();
    elsewhere["session_id"] = Value::from("00000000-0000-4000-8000-000000000009");
    assert_eq!(given(&elsewhere, home)?, None);
    // A file the store does not hold yet, and one whose name says it holds secrets, which the
    // index never reads: neither is told anything.
    for path in ["calc/fresh.py", "calc/credentials.py"] {
        std::fs::write(
            repo.join(path),
            "def add(a, b, c=0):\n    return a + b + c\n",
        )?;
        let mut other = add.clone();
        let native = repo.join(path).to_string_lossy().into_owned();
        other["tool_input"]["file_path"] = Value::from(native.clone());
        other["tool_response"]["filePath"] = Value::from(native);
        assert_eq!(given(&other, home)?, None, "{path}");
    }
    Ok(())
}
