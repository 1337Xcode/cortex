//! What the CLI's integration tests share: a daemon run on a temporary repository with an empty
//! home folder, its graph once built, a guard that stops a daemon a command started, and waiting
//! for a condition.

#![allow(
    dead_code,
    reason = "each test file uses a different part of these helpers"
)]

pub mod calc;
pub mod fake_daemon;
pub mod git;
pub mod hook_call;
pub mod init;
pub mod payloads;
pub mod receipts;

use std::error::Error;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cortex_core::ipc::ResponseBody;
use cortex_core::ipc::{
    GitReport, GraphReport, RequestBody, Role, StatusReport, UpkeepReport, WorktreeReport,
};
use cortex_core::{RepoId, RunFiles, UserFolder, canonical_root};
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
        Self::spawn(daemon_command(root, home))
    }

    /// Starts the daemon `command` runs, such as [`daemon_command`] with something changed.
    pub fn spawn(mut command: Command) -> TestResult<Self> {
        let mut child = command
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

/// Makes `command` see `home` as the user's home folder, read no git configuration but what is
/// there, and find Claude Code's configuration folder, transcripts included, in its default place.
pub fn isolate(command: &mut Command, home: &Path) {
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for name in [
        "CLAUDE_CONFIG_DIR",
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

/// How long a command's output may go on after the command has ended. Its pipes end when the
/// last process holding them lets go: the command at once, and a daemon it started never, if the
/// daemon took the pipes with it, as an agent reading a hook's output would then find.
pub const OUTPUT_ENDS_WITHIN: Duration = Duration::from_secs(20);

/// Runs `command` with `input` on its stdin, a pipe and no terminal, and reads what it printed
/// through pipes, as an agent reads a hook and a script reads a command.
///
/// # Errors
///
/// The command cannot run, or its output has not ended [`OUTPUT_ENDS_WITHIN`] after it did: a
/// process it started holds its pipes.
pub fn printed(command: &mut Command, input: &[u8]) -> TestResult<Printed> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = read_to_end_apart(child.stdout.take().ok_or("stdout is not piped")?);
    let stderr = read_to_end_apart(child.stderr.take().ok_or("stderr is not piped")?);
    let mut stdin = child.stdin.take().ok_or("stdin is not piped")?;
    // A command may have given up its input already, as a hook that refuses a payload does.
    let _ = std::io::Write::write_all(&mut stdin, input);
    drop(stdin);
    let status = child.wait()?;
    let ended = |output: mpsc::Receiver<std::io::Result<Vec<u8>>>, name: &str| {
        let bytes = output.recv_timeout(OUTPUT_ENDS_WITHIN).map_err(|_| {
            format!("the command's {name} did not end after it did: a process it started holds it")
        })??;
        TestResult::Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    Ok(Printed {
        is_success: status.success(),
        stdout: ended(stdout, "stdout")?,
        stderr: ended(stderr, "stderr")?,
    })
}

/// Reads `pipe` to its end on a thread of its own, and sends what it read.
fn read_to_end_apart(
    mut pipe: impl std::io::Read + Send + 'static,
) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = pipe.read_to_end(&mut bytes).map(|_| bytes);
        // The test may have given up waiting and dropped the receiver: nothing is lost.
        let _ = send.send(read);
    });
    receive
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

/// Stops a daemon a hook may still be starting for the repository whose run files are `files`:
/// asks it to stop until it says it will, for ten seconds, more than a daemon takes to bind, then
/// waits for it to let go of the repository. One that never starts, or already stopped, cannot be
/// asked, and the ten seconds pass.
pub fn stop_late_daemon(files: &RunFiles) {
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        let deadline = Instant::now() + Duration::from_secs(2);
        let stops = ask(files, Role::Cli, RequestBody::Shutdown, deadline)
            .is_ok_and(|answer| answer.body == ResponseBody::Ack);
        if stops {
            break;
        }
        std::thread::sleep(POLL_EVERY);
    }
    let _ = wait_for(|| Ok(!files.pid_path().exists()));
}

/// The report on the graph of the daemon of the repository at `root`, `home` being the home
/// folder, once it has built one: none before its first build.
///
/// # Errors
///
/// The daemon does not answer its status.
pub fn graph_of(root: &Path, home: &Path) -> TestResult<Option<GraphReport>> {
    Ok(status_report(root, home)?
        .graph
        .filter(|graph| graph.builds > 0))
}

/// Where the co-change miner of the daemon of the repository at `root`, `home` being the home
/// folder, is: none from a daemon that does not say.
///
/// # Errors
///
/// The daemon does not answer its status.
pub fn git_of(root: &Path, home: &Path) -> TestResult<Option<GitReport>> {
    Ok(status_report(root, home)?.git)
}

/// What the daemon of the repository at `root`, `home` being the home folder, read of its work
/// tree's uncommitted changes: none before its first look ended.
///
/// # Errors
///
/// The daemon does not answer its status.
pub fn worktree_of(root: &Path, home: &Path) -> TestResult<Option<WorktreeReport>> {
    Ok(status_report(root, home)?.worktree)
}

/// The status of the daemon of the repository at `root`, `home` being the home folder.
fn status_report(root: &Path, home: &Path) -> TestResult<StatusReport> {
    let files = UserFolder::in_home(home).run_files(&RepoId::of(&canonical_root(root)?));
    let deadline = Instant::now() + Duration::from_secs(10);
    match ask(&files, Role::Cli, RequestBody::Status, deadline)?.body {
        ResponseBody::Status(report) => Ok(report),
        other => Err(format!("the daemon answered its status with {other:?}").into()),
    }
}

/// What the last run of upkeep did, as the daemon of the repository at `root` reports it; none
/// before the first run.
///
/// # Errors
///
/// The daemon does not answer its status.
pub fn upkeep_of(root: &Path, home: &Path) -> TestResult<Option<UpkeepReport>> {
    Ok(status_report(root, home)?.upkeep)
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
