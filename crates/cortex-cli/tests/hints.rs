//! A read's hint, through the real binary as Claude Code runs it, on the repository the captured
//! payloads come from (pinned to medium mode): once the session has read `calc/ops.py`, a read of
//! some of its lines is told they were read already and are unchanged; another session's read is
//! told nothing, and nor is a read once the file has changed.
#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

mod common;

use std::path::Path;
use std::time::Duration;

use common::calc::{OPS, S1, captured, repository};
use common::hook_call::hook;
use common::{Daemon, TestResult, graph_of, wait_for};
use cortex_core::{ContentHash, EventKind, Identifier, RepoPath};
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

/// How many reads of `calc/ops.py` the store holds for session `session`.
fn reads_stored(root: &Path, session: &str) -> TestResult<usize> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let queries = reader.queries();
    let events = queries.events_of_session(&Identifier::new(session)?)?;
    Ok(events
        .iter()
        .filter(|row| {
            row.record.kind == EventKind::Read
                && row.record.path.as_ref().map(RepoPath::as_str) == Some("calc/ops.py")
        })
        .count())
}

/// Whether the store holds `calc/ops.py` as `text`.
fn holds_ops(root: &Path, text: &str) -> TestResult<bool> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let row = reader.queries().file(&RepoPath::new("calc/ops.py")?)?;
    Ok(row.is_some_and(|row| row.file.content_hash == Some(ContentHash::of(text.as_bytes()))))
}

/// The read of lines 2 to 5 of `calc/ops.py` is told those lines were read, once the session's
/// read of the whole file is stored: by hand, the captured range read asks from line 2 for 4
/// lines. Before that read, and from another session, nothing is told.
#[test]
fn a_range_read_before_is_told_so() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    std::fs::create_dir_all(repo.join(".cortex"))?;
    std::fs::write(repo.join(".cortex/config.toml"), "mode = \"medium\"\n")?;
    let daemon = Daemon::start(&repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home)?.is_some()))?;

    // The captured range read came from a session of its own: sent here as the whole read's.
    let mut range = captured("pre-tool-use-read-range.json", &repo)?;
    range["session_id"] = Value::from(S1);
    assert_eq!(given(&range, home)?, None);
    given(&captured("post-tool-use-read.json", &repo)?, home)?;
    wait_for(|| Ok(reads_stored(&repo, S1)? == 1))?;
    assert_eq!(
        given(&range, home)?.as_deref(),
        Some(
            "[cortex] You read lines 2-5 of this file earlier in this session; unchanged since.\n"
        )
    );

    let mut elsewhere = range.clone();
    elsewhere["session_id"] = Value::from("00000000-0000-4000-8000-000000000009");
    assert_eq!(given(&elsewhere, home)?, None);

    let changed = format!("{OPS}\n# changed\n");
    std::fs::write(repo.join("calc/ops.py"), &changed)?;
    wait_for(|| holds_ops(&repo, &changed))?;
    assert_eq!(given(&range, home)?, None);
    Ok(())
}

/// A tiny repository, as the captured one is unpinned, gets no hint: there Cortex only records.
#[test]
fn a_tiny_repository_gets_no_hint() -> TestResult {
    let (home, repo) = repository()?;
    let home = home.path();
    let daemon = Daemon::start(&repo, home)?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    wait_for(|| Ok(graph_of(&repo, home)?.is_some()))?;
    let mut range = captured("pre-tool-use-read-range.json", &repo)?;
    range["session_id"] = Value::from(S1);
    given(&captured("post-tool-use-read.json", &repo)?, home)?;
    wait_for(|| Ok(reads_stored(&repo, S1)? == 1))?;
    assert_eq!(given(&range, home)?, None);
    Ok(())
}
