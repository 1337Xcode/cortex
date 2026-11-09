//! `cortex consolidate` through the binary, against a real daemon: the daemon answers at once,
//! runs upkeep as soon as its writer is free, and `cortex status` then says what the run removed.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Daemon, Printed, TestResult, graph_of, isolate, printed, upkeep_of, wait_for};

/// Runs `cortex` with `args` in the repository at `root`, `home` being the home folder.
fn cortex(root: &Path, home: &Path, args: &[&str]) -> TestResult<Printed> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args).current_dir(root);
    isolate(&mut command, home);
    printed(&mut command, b"")
}

/// A fresh repository has nothing for upkeep to remove: asked for upkeep, the daemon says it will
/// run it; once it has, its status reports a run that removed nothing and finished, and `cortex
/// status` prints it.
#[test]
fn upkeep_runs_when_asked_and_the_status_says_so() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let root = repo.path();
    std::fs::write(root.join("main.py"), "def main():\n    return 0\n")?;
    let daemon = Daemon::start(root, home.path())?;
    daemon.first_line()?;
    wait_for(|| Ok(graph_of(root, home.path())?.is_some()))?;
    let home = home.path();
    let asked = cortex(root, home, &["consolidate"])?;
    assert!(asked.is_success, "{}", asked.stderr);
    assert_eq!(
        asked.stdout,
        "upkeep asked for: it runs as soon as the daemon is free; `cortex status` says what it \
         removed\n"
    );
    wait_for(|| Ok(upkeep_of(root, home)?.is_some()))?;
    let ran = upkeep_of(root, home)?.ok_or("no run")?;
    assert!(ran.is_done);
    assert_eq!(
        (
            ran.links,
            ran.interest,
            ran.facts,
            ran.events,
            ran.episodes,
            ran.positions
        ),
        (0, 0, 0, 0, 0, 0)
    );
    let status = cortex(root, home, &["status"])?;
    assert!(status.is_success, "{}", status.stderr);
    assert!(
        status
            .stdout
            .lines()
            .any(|line| line.starts_with("upkeep: last ran ")
                && line.ends_with(", nothing to remove; done")),
        "{}",
        status.stdout
    );
    Ok(())
}
