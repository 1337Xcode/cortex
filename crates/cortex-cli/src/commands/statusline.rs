//! `cortex statusline`: one line for Claude Code's status line, which runs a command with the
//! session's details as JSON on stdin after each response and shows what it prints. The line says
//! how many tokens Cortex added to the session and what the holdout comparison has measured, in the
//! words `cortex status` uses: `cortex: 1,240 tokens added · savings not measured yet (12/30 held
//! out, 40/30 with Cortex)`.
//!
//! It only reads: the repository's store is opened read-only, no daemon is started and nothing is
//! written. A status line must never get in the way, so any problem (stdin that is not the
//! session's JSON, a folder with no store, a store that cannot be read, even a panic) prints
//! nothing and exits 0.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use cortex_core::Identifier;
use cortex_meter::{Recorded, ReportError, Selection, recorded};
use cortex_store::Reader;
use serde::Deserialize;

use super::receipts::grouped;
use super::savings;
use crate::output;

/// The most bytes read from stdin. Claude Code's payload is a few kilobytes; more is not it.
const MAX_PAYLOAD_BYTES: u64 = 64 * 1024;

/// The arguments of `cortex statusline`: none, the session comes on stdin.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {}

/// The parts of Claude Code's status line payload this reads; the rest is ignored.
#[derive(Debug, Deserialize)]
struct Payload {
    /// The session's id.
    session_id: String,
    /// The folder the session works in.
    cwd: Option<PathBuf>,
    /// The session's folders.
    workspace: Option<Workspace>,
}

/// The session's folders.
#[derive(Debug, Deserialize)]
struct Workspace {
    /// Where Claude Code was started.
    project_dir: Option<PathBuf>,
    /// Where the session works now.
    current_dir: Option<PathBuf>,
}

/// Prints the status line for the session on stdin, or nothing when it cannot be made.
///
/// # Errors
///
/// None: every failure prints nothing.
pub(crate) fn run(_: &Args) -> anyhow::Result<()> {
    // A status line that fails prints nothing, a panic included: Claude Code shows whatever is
    // printed, and an error message there helps nobody.
    let line = std::panic::catch_unwind(|| line_of(std::io::stdin())).unwrap_or(None);
    if let Some(line) = line {
        // A closed stdout leaves nothing to tell.
        let _ = output::line(&line);
    }
    Ok(())
}

/// The status line for the payload `input` holds, or none when any part cannot be read.
fn line_of(input: impl std::io::Read) -> Option<String> {
    let mut bytes = Vec::new();
    input.take(MAX_PAYLOAD_BYTES).read_to_end(&mut bytes).ok()?;
    let payload: Payload = serde_json::from_slice(&bytes).ok()?;
    let session = Identifier::new(&payload.session_id).ok()?;
    let folder = folder_of(&payload)?;
    let home = std::env::home_dir();
    let located = cortex_hook::locate_marked(&folder, home.as_deref())?;
    let reader = Reader::open(&located.repository_root().join(".cortex"), 0).ok()?;
    let queries = reader.queries();
    let (added, report) = match recorded(queries, &Selection::Session(session), None) {
        Ok(report) => (added_of(&report), report),
        // A session not stored yet has had nothing added; the comparison is the store's.
        Err(ReportError::NoSession(_)) => {
            let report = recorded(queries, &Selection::StartedSince(i64::MAX), None).ok()?;
            (0, report)
        }
        Err(_) => return None,
    };
    Some(format!(
        "cortex: {} tokens added · {}",
        grouped(added),
        savings::short(&report.holdout)
    ))
}

/// The folder the session works in: where Claude Code was started, else where it works now.
fn folder_of(payload: &Payload) -> Option<PathBuf> {
    let workspace = payload.workspace.as_ref();
    workspace
        .and_then(|workspace| workspace.project_dir.clone())
        .or_else(|| workspace.and_then(|workspace| workspace.current_dir.clone()))
        .or_else(|| payload.cwd.clone())
        .filter(|folder| Path::is_dir(folder))
}

/// The tokens Cortex added to the one session `report` holds.
fn added_of(report: &Recorded) -> u64 {
    report
        .sessions
        .iter()
        .map(|session| session.injected_tokens)
        .sum()
}
