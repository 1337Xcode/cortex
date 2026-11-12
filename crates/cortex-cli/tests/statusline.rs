//! `cortex statusline` through the real binary: Claude Code's status line payload on stdin, one
//! line out naming the tokens Cortex added to the session and what the holdout comparison
//! measured; nothing at all, and success, on anything it cannot read.
//!
//! The payload follows the shape Claude Code's status line documentation gives (read 2026-11-12),
//! every field it lists present, so that the fields Cortex does not read are shown to be ignored.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Printed, TestResult, isolate, printed};
use cortex_core::{ContentHash, HookKind, Identifier, RepoId, Ulid, canonical_root};
use cortex_store::{InjectionRecord, SessionRecord, TaskRecord, Writer, WriterOptions};
use serde_json::json;

/// Claude Code's status line payload for session `session`, started in `folder`.
fn payload(session: &str, folder: &Path) -> Vec<u8> {
    let folder = folder.to_string_lossy();
    json!({
        "cwd": folder,
        "session_id": session,
        "session_name": "my-session",
        "transcript_path": "/path/to/transcript.jsonl",
        "model": {"id": "claude-opus-5-5", "display_name": "Opus"},
        "workspace": {
            "current_dir": folder,
            "project_dir": folder,
            "added_dirs": [],
        },
        "version": "2.1.318",
        "output_style": {"name": "default"},
        "cost": {
            "total_cost_usd": 0.01234,
            "total_duration_ms": 45000,
            "total_api_duration_ms": 2300,
            "total_lines_added": 156,
            "total_lines_removed": 23,
        },
        "context_window": {
            "total_input_tokens": 15500,
            "total_output_tokens": 1200,
            "context_window_size": 200000,
            "used_percentage": 8,
            "remaining_percentage": 92,
            "current_usage": null,
        },
        "exceeds_200k_tokens": false,
        "fast_mode": false,
    })
    .to_string()
    .into_bytes()
}

/// Runs `cortex statusline` in `folder` with `home` as the home folder, `input` on stdin.
fn statusline(folder: &Path, home: &Path, input: &[u8]) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.arg("statusline").current_dir(folder);
    isolate(&mut command, home);
    printed(&mut command, input)
}

/// An injection of `tokens` for session `s1`'s task `task` at the prompt, gated out or held out as
/// `(is_gated_out, is_holdout)` says, its text written so that one not kept back reached the agent.
fn injection(
    task: Ulid,
    tokens: u32,
    (is_gated_out, is_holdout): (bool, bool),
) -> TestResult<InjectionRecord> {
    Ok(InjectionRecord {
        task_id: Some(task),
        session_id: Identifier::new("s1")?,
        hook: HookKind::Prompt,
        at_ms: 2_000,
        files: Vec::new(),
        tokens,
        confidence: None,
        is_gated_out,
        is_holdout,
        text_hash: Some(ContentHash::of(b"a work order")),
        features: None,
    })
}

/// Worked by hand: session `s1` has one task, decided treated, given three work orders of 120, 50
/// and 30 tokens; the 50 was gated out and the 30 held out, so only 120 reached the agent. The
/// arms are 0 held out and 1 treated, against 30 each, so no saving is stated. A session the store
/// does not hold yet has had nothing added. Stdin that is not the payload, a payload naming no
/// session, and a folder with no store each print nothing and succeed.
#[test]
fn the_status_line_names_the_tokens_added_and_states_no_unmeasured_saving() -> TestResult {
    let home = tempfile::tempdir()?;
    let repo = tempfile::tempdir()?;
    let root = canonical_root(repo.path())?;
    let mut writer = Writer::open(&WriterOptions {
        folder: &root.join(".cortex"),
        repo_id: &RepoId::of(&root),
        mmap_bytes: 0,
        now_ms: 1,
    })?;
    let task = Ulid::from_seed(1, b"statusline");
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: Identifier::new("s1")?,
        agent: cortex_core::Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 1_000,
    })?;
    transaction.start_task(&TaskRecord {
        task_id: task,
        session_id: Identifier::new("s1")?,
        started_at_ms: 1_500,
        prompt_hash: None,
        prompt_excerpt: None,
        holdout: Some(false),
    })?;
    for (tokens, kept_back) in [
        (120, (false, false)),
        (50, (true, false)),
        (30, (false, true)),
    ] {
        transaction.record_injection(&injection(task, tokens, kept_back)?)?;
    }
    transaction.commit()?;
    drop(writer);

    let known = statusline(&root, home.path(), &payload("s1", &root))?;
    assert!(known.is_success, "{}", known.stderr);
    assert_eq!(
        known.stdout,
        "cortex: 120 tokens added · savings not measured yet (0/30 held out, 1/30 with Cortex)\n"
    );
    let new = statusline(&root, home.path(), &payload("s9", &root))?;
    assert_eq!(
        new.stdout,
        "cortex: 0 tokens added · savings not measured yet (0/30 held out, 1/30 with Cortex)\n"
    );
    let elsewhere = tempfile::tempdir()?;
    for (folder, input) in [
        (root.as_path(), b"not the payload".to_vec()),
        (root.as_path(), br#"{"cwd": "."}"#.to_vec()),
        (elsewhere.path(), payload("s1", elsewhere.path())),
    ] {
        let silent = statusline(folder, home.path(), &input)?;
        assert!(silent.is_success, "{}", silent.stderr);
        assert_eq!(silent.stdout, "", "{}", String::from_utf8_lossy(&input));
        assert_eq!(silent.stderr, "");
    }
    Ok(())
}
