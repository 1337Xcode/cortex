//! The keep-list a compaction's summary is given, through the real binary as Claude Code runs it,
//! on the repository the captured payloads come from: with `compaction.keep_list` on, once the
//! session's task edited `calc/ops.py` and was warned of its callers and tests, the captured
//! `PreCompact` prints the task's working set as plain text, not hook JSON, recorded as given to
//! the task with the hash of what was printed; the start after the compaction still gets its
//! recap. With the key left at its default, for a held-out task, and in a tiny repository, the
//! compaction prints nothing, and is recorded all the same.
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
use cortex_core::{ContentHash, EventKind, HookKind, Identifier};
use cortex_store::{Queries, Reader};
use serde_json::Value;

/// What the captured compaction prints, by hand: the keep-list's header, then the lines the recap
/// after it gives (see the recap's own test): the task edited `calc/ops.py`; its edit was warned of
/// the calls to `add` in `main.py` and `tests/test_ops.py`, then of the tests: the latest first,
/// each file once. The prompt is not kept: the configuration keeps no excerpt.
const KEEP_LIST: &str = "[cortex] Keep in the summary that the task in progress:\n\
    - edited calc/ops.py\n\
    - was warned of tests/test_ops.py, the tests of calc/ops.py; not read or edited since\n\
    - was warned of main.py, which calls a signature this task changed; not read or edited since\n";

/// The recap the start after the compaction gets: the same lines under its own header.
const RECAP: &str = "[cortex] Before the compaction, the task in progress:\n\
    - edited calc/ops.py\n\
    - was warned of tests/test_ops.py, the tests of calc/ops.py; not read or edited since\n\
    - was warned of main.py, which calls a signature this task changed; not read or edited since\n";

/// The configuration of a repository pinned to medium mode with no task held out, and `extra`.
fn medium(extra: &str) -> String {
    format!("mode = \"medium\"\n[holdout]\nshare = 0\n{extra}")
}

/// Runs `cortex hook` on `payload` as Claude Code does, and returns what it gave the agent: the
/// plain text a compaction prints, or the context of any other hook's JSON.
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
    if event == "PreCompact" {
        call.plain(&event)
    } else {
        call.context(&event)
    }
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

/// Starts the daemon on `repo` with `config`, opens the captured session's task, changes `add`'s
/// header in it, as the recap's test does, and waits until the store holds the edit; and, when
/// `is_warned`, the edit's three warnings.
fn edited(repo: &Path, home: &Path, (config, is_warned): (&str, bool)) -> TestResult<Daemon> {
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), config)?;
    let daemon = Daemon::start(repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(repo, home)?.is_some()))?;
    given(&prompt(repo, "Let add take a third number.")?, home)?;
    let session = Identifier::new(S1)?;
    wait_for(|| read(repo, |queries| Ok(queries.open_task(&session)?.is_some())))?;
    let (old, new) = ("def add(a, b):", "def add(a, b, c=0):");
    std::fs::write(repo.join("calc/ops.py"), OPS.replace(old, new))?;
    let mut edit = captured("post-tool-use-edit.json", repo)?;
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
    given(&edit, home)?;
    wait_for(|| Ok(stored(repo, EventKind::Edit)? == 1))?;
    if is_warned {
        wait_for(|| {
            read(repo, |queries| {
                let task = queries.open_task(&session)?.ok_or("the task")?;
                Ok(queries.warnings_of_task(task.record.task_id)?.len() == 3)
            })
        })?;
    }
    Ok(daemon)
}

/// With the key on, the compaction prints the keep-list as plain text, and the store records it as
/// given to the task at the compaction, with the hash of the text printed; the start after the
/// compaction is told its recap.
#[test]
fn a_compaction_is_given_its_keep_list() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = edited(
        &repo,
        home,
        (&medium("[compaction]\nkeep_list = true\n"), true),
    )?;
    let compaction = captured("pre-compact-manual.json", &repo)?;
    assert_eq!(given(&compaction, home)?.as_deref(), Some(KEEP_LIST));
    assert!(KEEP_LIST.chars().count() <= 600);
    let start = captured("session-start-compact.json", &repo)?;
    assert_eq!(given(&start, home)?.as_deref(), Some(RECAP));
    let session = Identifier::new(S1)?;
    wait_for(|| {
        read(&repo, |queries| {
            let task = queries.open_task(&session)?.ok_or("the task")?;
            let kept: Vec<_> = queries
                .injections_of_session(&session)?
                .into_iter()
                .map(|row| row.record)
                .filter(|record| record.hook == HookKind::Compact)
                .collect();
            Ok(match kept.as_slice() {
                [record] => {
                    assert_eq!(record.task_id, Some(task.record.task_id));
                    assert_eq!(
                        record.text_hash,
                        Some(ContentHash::of(KEEP_LIST.as_bytes()))
                    );
                    true
                }
                [] => false,
                more => return Err(format!("{} keep-lists recorded", more.len()).into()),
            })
        })
    })?;
    wait_for(|| Ok(stored(&repo, EventKind::Compact)? == 1))?;
    drop(daemon);
    Ok(())
}

/// With the key left at its default, for a task held out (every task held out) and in a tiny
/// repository, the compaction prints nothing; the daemon records it all the same.
#[test]
fn a_compaction_is_given_nothing_unless_the_key_is_on() -> TestResult {
    let on = "[compaction]\nkeep_list = true\n";
    for (config, is_warned) in [
        (medium(""), true),
        (
            format!("mode = \"medium\"\n[holdout]\nshare = 1\n{on}"),
            false,
        ),
        (
            format!("mode = \"tiny\"\n[holdout]\nshare = 0\n{on}"),
            false,
        ),
    ] {
        let (home, repo) = repository()?;
        let home = home.path();
        let daemon = edited(&repo, home, (&config, is_warned))?;
        let compaction = captured("pre-compact-manual.json", &repo)?;
        assert_eq!(given(&compaction, home)?, None, "{config}");
        wait_for(|| Ok(stored(&repo, EventKind::Compact)? == 1))?;
        drop(daemon);
    }
    Ok(())
}

/// Outside a task, before the session's first prompt, the compaction prints nothing with the key
/// on; the daemon records it all the same.
#[test]
fn a_compaction_outside_a_task_is_given_nothing() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    std::fs::create_dir_all(repo.join(".cortex"))?;
    let config = medium("[compaction]\nkeep_list = true\n");
    std::fs::write(repo.join(".cortex/config.toml"), config)?;
    let daemon = Daemon::start(&repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home)?.is_some()))?;
    let compaction = captured("pre-compact-manual.json", &repo)?;
    assert_eq!(given(&compaction, home)?, None);
    wait_for(|| Ok(stored(&repo, EventKind::Compact)? == 1))?;
    drop(daemon);
    Ok(())
}
