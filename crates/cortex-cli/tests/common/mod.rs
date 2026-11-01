//! What the CLI's integration tests share: a daemon run on a temporary repository with an empty
//! home folder, a guard that stops a daemon a command started, and waiting for a condition.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

pub mod fake_daemon;
pub mod init;
pub mod payloads;

use std::error::Error;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cortex_core::ipc::{RequestBody, Role};
use cortex_core::{RepoId, UserFolder, canonical_root};
use cortex_daemon::is_running;
use cortex_hook::{ClientError, ask};

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

/// What a command printed, and whether it succeeded.
pub struct Printed {
    /// Whether it succeeded.
    pub is_success: bool,
    /// Its stdout.
    pub stdout: String,
    /// Its stderr.
    pub stderr: String,
}

/// Runs `command`, its stdin an empty pipe, which is no terminal, and its output written to files in
/// `folder` rather than pipes: on Windows a daemon the command starts holds the command's own
/// handles, and a pipe would stay open until that daemon stops.
///
/// # Errors
///
/// The files cannot be made or read, or the command cannot run.
pub fn printed_to_files(command: &mut Command, folder: &Path) -> TestResult<Printed> {
    let stdout = folder.join("printed-stdout.txt");
    let stderr = folder.join("printed-stderr.txt");
    command
        .stdin(Stdio::piped())
        .stdout(File::create(&stdout)?)
        .stderr(File::create(&stderr)?);
    let mut child = command.spawn()?;
    drop(child.stdin.take());
    let status = child.wait()?;
    Ok(Printed {
        is_success: status.success(),
        stdout: std::fs::read_to_string(&stdout)?,
        stderr: std::fs::read_to_string(&stderr)?,
    })
}

/// Stops, when dropped, the daemon a command started detached for the repository at `root`, the
/// user's home being `home`, and waits for it to let go of the repository, so that its folders can
/// be removed. Declared after the folders, it is dropped before them, a failed test included.
pub struct StopsDaemon {
    /// The repository.
    root: PathBuf,
    /// The home folder.
    home: PathBuf,
}

impl StopsDaemon {
    /// A guard for the daemon of the repository at `root` with `home` as the home folder.
    pub fn of(root: &Path, home: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            home: home.to_path_buf(),
        }
    }

    /// Stops the daemon, if one runs, and waits for it to let go of the repository.
    ///
    /// # Errors
    ///
    /// The repository's folder cannot be found, the daemon cannot be asked to stop, or it does not
    /// stop in time.
    pub fn stop(&self) -> TestResult {
        let user = UserFolder::in_home(&self.home);
        let repo_id = RepoId::of(&canonical_root(&self.root)?);
        if !is_running(&user, &repo_id)? {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        // A daemon that stopped between the look and the request needs nothing more.
        match ask(
            &user.run_files(&repo_id),
            Role::Cli,
            RequestBody::Shutdown,
            deadline,
        ) {
            Ok(_) | Err(ClientError::NotRunning) => {}
            Err(error) => return Err(error.into()),
        }
        wait_for(|| Ok(!is_running(&user, &repo_id)?))
    }
}

impl Drop for StopsDaemon {
    /// Stops the daemon; a test that already failed has its own error to show.
    fn drop(&mut self) {
        drop(self.stop());
    }
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
