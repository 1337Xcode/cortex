//! A daemon's life: it stops on its own once it has been idle as long as its configuration says,
//! a request of any kind puts the stop off, and a command of a newer build replaces it.

mod common;

use std::process::Command;
use std::time::{Duration, Instant};

use common::fake_daemon::{FakeDaemon, Script, kinds};
use common::{Daemon, POLL_EVERY, TestResult, isolate, printed, wait_for};
use cortex_core::ipc::{CORTEX_VERSION, RequestBody, ResponseBody, Role};
use cortex_core::{RepoId, UserFolder, canonical_root};
use cortex_daemon::is_running;
use cortex_hook::ask;

/// How long a daemon set to stop after one idle minute is left alone before the request that puts
/// its stop off.
const FIRST_QUIET: Duration = Duration::from_secs(40);

/// One minute, the idle time the test's daemon is set to.
const IDLE: Duration = Duration::from_secs(60);

/// How long after its due time the daemon may take to notice and stop: its stop gives open
/// connections one second, and a busy machine is slow.
const STOP_WITHIN: Duration = Duration::from_secs(30);

/// A daemon set to stop after one idle minute is asked for its status 40 s after it is ready, and
/// so is still running a minute after it was ready, when it would have stopped without the
/// request; it stops, exiting with 0, between a minute after the request and half a minute later.
/// The request is the only use: no event, no file change.
#[test]
#[ignore = "takes two minutes; `cargo xtask verify --full` runs it"]
fn a_request_puts_off_the_idle_stop() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(repo.path().join("app.py"), "def app():\n    pass\n")?;
    std::fs::create_dir_all(repo.path().join(".cortex"))?;
    std::fs::write(
        repo.path().join(".cortex").join("config.toml"),
        "[daemon]\nidle_shutdown_minutes = 1\n",
    )?;
    let mut daemon = Daemon::start(repo.path(), home.path())?;
    let line = daemon.first_line()?;
    assert!(line.starts_with("cortex: daemon ready"), "{line}");
    let ready = Instant::now();
    std::thread::sleep(FIRST_QUIET);
    let mut status = Command::new(env!("CARGO_BIN_EXE_cortex"));
    status.args(["status", "--root"]).arg(repo.path());
    isolate(&mut status, home.path());
    // The daemon takes the request somewhere between these two moments.
    let asking = Instant::now();
    let output = status.output()?;
    let asked = Instant::now();
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.starts_with("daemon: running"), "{stdout}");
    std::thread::sleep((ready + IDLE + Duration::from_secs(10)).saturating_duration_since(asked));
    assert_eq!(
        daemon.child.try_wait()?,
        None,
        "stopped a minute after ready"
    );
    let deadline = asked + IDLE + STOP_WITHIN;
    let exit = loop {
        if let Some(exit) = daemon.child.try_wait()? {
            break exit;
        }
        assert!(
            Instant::now() < deadline,
            "still running past its idle time"
        );
        std::thread::sleep(POLL_EVERY);
    };
    assert!(
        asking.elapsed() >= IDLE,
        "stopped {:?} after the request was sent",
        asking.elapsed()
    );
    assert_eq!(exit.code(), Some(0));
    Ok(())
}

/// `cortex status` that finds a daemon of an older Cortex asks it to stop, waits for it to let go
/// of the repository, starts a daemon of this build in its place and reports that one. The new
/// daemon runs detached, as one a hook starts, and is stopped at the end.
#[test]
fn a_command_replaces_an_older_daemon() -> TestResult {
    let repo = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    std::fs::write(repo.path().join("app.py"), "def app():\n    pass\n")?;
    let older = FakeDaemon::start(repo.path(), home.path(), Script::Welcome(&["1.0.0"]))?;
    let mut status = Command::new(env!("CARGO_BIN_EXE_cortex"));
    status.args(["status", "--root"]).arg(repo.path());
    isolate(&mut status, home.path());
    // Read through pipes, as a script reads the command: they end with the command, while the
    // daemon it started goes on.
    let ran = printed(&mut status, b"")?;
    let stdout = ran.stdout;
    assert!(ran.is_success, "{stdout}{}", ran.stderr);
    assert_eq!(
        kinds(&older.requests()?),
        ["hello", "status", "hello", "shutdown"]
    );
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some("daemon: replaced a daemon of Cortex 1.0.0"),
        "{stdout}"
    );
    let running = lines.next().unwrap_or_default();
    assert!(
        running.starts_with("daemon: running (process ")
            && running.contains(&format!("Cortex {CORTEX_VERSION},")),
        "{stdout}"
    );
    let user = UserFolder::in_home(home.path());
    let repo_id = RepoId::of(&canonical_root(repo.path())?);
    let deadline = Instant::now() + Duration::from_secs(10);
    let stop = ask(
        &user.run_files(&repo_id),
        Role::Cli,
        RequestBody::Shutdown,
        deadline,
    )?;
    assert_eq!(stop.body, ResponseBody::Ack);
    wait_for(|| Ok(!is_running(&user, &repo_id)?))?;
    Ok(())
}
