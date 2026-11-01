//! What the CLI's integration tests share: a daemon run on a temporary repository with an empty
//! home folder, and waiting for a condition.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

pub mod fake_daemon;

use std::error::Error;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Result type for these tests: any failure fails the test.
pub type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Long enough for a debug build to load every grammar and index a few files.
pub const READY_WITHIN: Duration = Duration::from_secs(120);

/// Long enough for the watcher's 200 ms of quiet, or the crawler's first batch, and an index of a
/// few files.
pub const SEEN_WITHIN: Duration = Duration::from_secs(60);

/// How often a condition is checked while waiting for it.
pub const POLL_EVERY: Duration = Duration::from_millis(50);

/// A running daemon, stopped when dropped, and the lines it writes to stderr.
pub struct Daemon {
    /// The process.
    pub child: Child,
    /// Its stderr, a line at a time.
    lines: mpsc::Receiver<String>,
}

impl Daemon {
    /// Starts `cortex daemon` on `root`, with `home` as the user's home folder.
    pub fn start(root: &Path, home: &Path) -> TestResult<Self> {
        let mut child = daemon_command(root, home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child.stderr.take().ok_or("stderr is not piped")?;
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self { child, lines })
    }

    /// The first line the daemon writes, waited for.
    pub fn first_line(&self) -> TestResult<String> {
        Ok(self.lines.recv_timeout(READY_WITHIN)?)
    }

    /// The next line the daemon has written by now, if any, not waited for.
    pub fn line_by_now(&self) -> Option<String> {
        self.lines.try_recv().ok()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // The test is over: a daemon that already stopped cannot be killed, and that is fine.
        drop(self.child.kill());
        drop(self.child.wait());
    }
}

/// `cortex daemon --root <root>` with `home` as the user's home folder, and git's configuration
/// read from there only.
pub fn daemon_command(root: &Path, home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(["daemon", "--root"]).arg(root);
    isolate(&mut command, home);
    command
}

/// Makes `command` see `home` as the user's home folder and read no git configuration but what is
/// there.
pub fn isolate(command: &mut Command, home: &Path) {
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for name in [
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "XDG_CONFIG_HOME",
        "HOMEDRIVE",
        "HOMEPATH",
    ] {
        command.env_remove(name);
    }
}

/// A PATH for a test's command: `home`'s `bin` folder, where a test puts the programs it stands in
/// for, then the folders of this machine's PATH that hold one of `programs` (such as `git`), and
/// on Windows the system's own folder, whose command shell runs a `.cmd` stand-in.
///
/// # Errors
///
/// A folder whose name holds the PATH's separator.
pub fn test_path(home: &Path, programs: &[&str]) -> TestResult<OsString> {
    let mut folders = vec![home.join("bin")];
    let names: Vec<String> = programs
        .iter()
        .map(|program| format!("{program}{}", std::env::consts::EXE_SUFFIX))
        .collect();
    let path = std::env::var_os("PATH").unwrap_or_default();
    for folder in std::env::split_paths(&path) {
        let holds = names.iter().any(|name| folder.join(name).is_file());
        if holds && !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    if let Some(system) = std::env::var_os("SystemRoot") {
        folders.push(Path::new(&system).join("System32"));
    }
    Ok(std::env::join_paths(folders)?)
}

/// Writes a stand-in for Claude Code in `home`'s `bin` folder that answers `--version` as Claude
/// Code `version` does, and returns its path.
pub fn stand_in_claude(home: &Path, version: &str) -> TestResult<PathBuf> {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin)?;
    if cfg!(windows) {
        let claude = bin.join("claude.cmd");
        std::fs::write(&claude, format!("@echo {version} (Claude Code)\r\n"))?;
        return Ok(claude);
    }
    let claude = bin.join("claude");
    std::fs::write(
        &claude,
        format!("#!/bin/sh\necho '{version} (Claude Code)'\n"),
    )?;
    #[cfg(unix)]
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    Ok(claude)
}

/// Waits until `condition` holds, for at most [`SEEN_WITHIN`].
pub fn wait_for(condition: impl Fn() -> TestResult<bool>) -> TestResult {
    let start = Instant::now();
    while !condition()? {
        if start.elapsed() > SEEN_WITHIN {
            return Err("the condition did not hold in time".into());
        }
        std::thread::sleep(POLL_EVERY);
    }
    Ok(())
}
