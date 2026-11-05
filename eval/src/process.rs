//! Running a program the runner waits for, its output written to a file and never read through a
//! pipe, and stopped when it outlasts its time.
//!
//! The output stays in the run's folder for whoever looks into a run later. A file also has no end
//! to wait for: a pipe ends only when every process holding it lets go, and the commands the runner
//! starts leave processes running (the daemon `cortex init` starts, whatever an agent's tools
//! start), so a pipe would tie the runner to what they inherit.

use std::fs::File;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

/// How often a running program is asked whether it has ended.
const POLL_EVERY: Duration = Duration::from_millis(200);

/// How many of a log's last lines an error message quotes.
const TAIL_LINES: usize = 20;

/// Runs `command` with no input and its output and errors written to `log`, and waits up to
/// `within` for it to end.
///
/// # Errors
///
/// The log cannot be made, the program cannot be started, or it outlasts `within`, in which case
/// it is stopped.
pub(crate) fn run_logged(
    mut command: Command,
    log: &Path,
    within: Duration,
) -> anyhow::Result<ExitStatus> {
    let out = File::create(log).with_context(|| format!("making {}", log.display()))?;
    let err = out
        .try_clone()
        .with_context(|| format!("sharing {}", log.display()))?;
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .with_context(|| format!("starting {program}"))?;
    let until = Instant::now() + within;
    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("waiting for {program}"))?
        {
            return Ok(status);
        }
        if Instant::now() >= until {
            child
                .kill()
                .with_context(|| format!("stopping {program}"))?;
            child.wait().with_context(|| format!("reaping {program}"))?;
            bail!(
                "{program} ran longer than {} s and was stopped; its output is in {}",
                within.as_secs(),
                log.display()
            );
        }
        std::thread::sleep(POLL_EVERY);
    }
}

/// The last lines of the log `log`, for an error message.
pub(crate) fn tail_of(log: &Path) -> String {
    std::fs::read(log).map_or_else(
        |error| format!("(the log {} could not be read: {error})", log.display()),
        |bytes| {
            let text = String::from_utf8_lossy(&bytes);
            let lines: Vec<&str> = text.lines().collect();
            let from = lines.len().saturating_sub(TAIL_LINES);
            lines.get(from..).unwrap_or_default().join("\n")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A git command: git is a program every run already needs.
    fn git(args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command.args(args);
        command
    }

    /// A program's output lands in its log, and its exit status comes back.
    #[test]
    fn the_output_lands_in_the_log() -> anyhow::Result<()> {
        let folder = tempfile::tempdir()?;
        let log = folder.path().join("git.log");
        let status = run_logged(git(&["--version"]), &log, Duration::from_secs(60))?;
        assert!(status.success());
        assert!(std::fs::read_to_string(&log)?.starts_with("git version "));
        let failed = run_logged(git(&["no-such-command"]), &log, Duration::from_secs(60))?;
        assert!(!failed.success());
        Ok(())
    }

    /// A program that outlasts its time is stopped and reported.
    #[test]
    fn a_program_past_its_time_is_stopped() -> anyhow::Result<()> {
        let folder = tempfile::tempdir()?;
        let log = folder.path().join("wait.log");
        let started = Instant::now();
        let outcome = run_logged(sleeper(), &log, Duration::from_millis(500));
        assert!(outcome.is_err());
        assert!(started.elapsed() < Duration::from_secs(20));
        Ok(())
    }

    /// A program that runs for about a minute.
    #[cfg(windows)]
    fn sleeper() -> Command {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"]);
        command
    }

    /// A program that runs for about a minute.
    #[cfg(not(windows))]
    fn sleeper() -> Command {
        let mut command = Command::new("sleep");
        command.arg("60");
        command
    }
}
