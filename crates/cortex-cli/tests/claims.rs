//! `cortex claims` through the binary, against a real daemon: a claim refused while no agent
//! session is at work; claims made for the session at work once its tool call is recorded, listed
//! with its agent, released by file and then all at once; and a name that is no file refused
//! before the daemon is asked.

mod common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use common::hook_call::hook;
use common::payloads::payload;
use common::{Daemon, Printed, TestResult, graph_of, isolate, printed, wait_for};
use cortex_core::Identifier;
use cortex_store::Reader;

/// The session the captured read belongs to.
const SESSION: &str = "00000000-0000-4000-8000-000000000001";

/// Runs `cortex` with `args` in the repository at `root`, `home` being the home folder.
fn cortex(root: &Path, home: &Path, args: &[&str]) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).current_dir(root);
    isolate(&mut command, home);
    printed(&mut command, b"")
}

/// What `cortex` printed on stdout with `args`, having succeeded.
fn said(root: &Path, home: &Path, args: &[&str]) -> TestResult<String> {
    let printed = cortex(root, home, args)?;
    if !printed.is_success {
        return Err(format!("cortex {args:?} failed: {}", printed.stderr).into());
    }
    Ok(printed.stdout)
}

/// Each claim `cortex claims list` prints, without how long ago it was made, which depends on
/// how fast the test runs: each line is the file, the agent and the session, then `, claimed `
/// and an age ending in `ago`.
fn claims(root: &Path, home: &Path) -> TestResult<Vec<String>> {
    said(root, home, &["claims", "list"])?
        .lines()
        .map(|line| {
            let (claim, age) = line
                .split_once(", claimed ")
                .ok_or_else(|| format!("not a claim: {line}"))?;
            if !age.ends_with(" ago") {
                return Err(format!("not an age: {line}").into());
            }
            Ok(claim.to_owned())
        })
        .collect()
}

/// Worked by hand. With no session at work, a claim is refused in the daemon's words. Once Claude
/// Code's session reads `calc/ops.py` (the captured read), claiming `calc/ops.py` and
/// `calc/tax.py` claims two files for it, listed by file with its agent and the first 8
/// characters of its id, and how long ago. Releasing `calc/ops.py` releases one claim; releasing all, the other;
/// releasing again, none. A name that is not a file is refused before the daemon is asked.
#[test]
fn claims_are_made_for_the_session_at_work_listed_and_released() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::create_dir_all(root.join("calc"))?;
    std::fs::write(
        root.join("calc/ops.py"),
        "def add(a, b):\n    return a + b\n",
    )?;
    std::fs::write(root.join("calc/tax.py"), "def rate():\n    return 1\n")?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some()))?;
    let home = home.path();

    let refused = cortex(root, home, &["claims", "add", "calc/ops.py"])?;
    assert!(!refused.is_success, "{}", refused.stdout);
    assert_eq!(
        refused.stderr,
        "cortex: error: no agent session is at work in this repository: a claim is held by the \
         session that makes it, so make it from the agent's own shell\n"
    );

    let read = payload("2.1.318", "post-tool-use-read.json", root)?;
    hook(
        &["--agent", "claude", "--event", "PostToolUse"],
        read,
        Duration::ZERO,
        home,
    )?;
    let session = Identifier::new(SESSION)?;
    wait_for(|| {
        let reader = Reader::open(&root.join(".cortex"), 0)?;
        let seen = reader
            .queries()
            .sessions_seen(std::slice::from_ref(&session))?;
        Ok(seen
            .first()
            .is_some_and(|seen| seen.last_event_ms.is_some()))
    })?;

    let both = ["claims", "add", "calc/ops.py", "calc/tax.py"];
    assert_eq!(
        said(root, home, &both)?,
        "claimed 2 files for the Claude Code session 00000000\n"
    );
    assert_eq!(
        claims(root, home)?,
        [
            "calc/ops.py  Claude Code session 00000000",
            "calc/tax.py  Claude Code session 00000000"
        ]
    );
    assert_eq!(
        said(root, home, &["claims", "release", "calc/ops.py"])?,
        "released 1 claim\n"
    );
    assert_eq!(
        claims(root, home)?,
        ["calc/tax.py  Claude Code session 00000000"]
    );
    assert_eq!(
        said(root, home, &["claims", "release"])?,
        "released 1 claim\n"
    );
    assert_eq!(
        said(root, home, &["claims", "release"])?,
        "no claims to release\n"
    );
    assert_eq!(said(root, home, &["claims", "list"])?, "no claims\n");

    let none = cortex(root, home, &["claims", "add", "calc/none.py"])?;
    assert!(!none.is_success, "{}", none.stdout);
    assert_eq!(
        none.stderr,
        "cortex: error: calc/none.py: a claim is on a file, and this is none\n"
    );
    Ok(())
}
