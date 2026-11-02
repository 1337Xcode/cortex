//! Running another program Cortex needs an answer from (git for a repository's history and the
//! global gitignore, `whoami` for the daemon, Claude Code's version and the hook's probes for
//! `cortex doctor`), bounded in time and in the output read.

use std::io::{self, Read, Write as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// How often a running program is checked on.
const POLL_EVERY: Duration = Duration::from_millis(10);

/// What a program printed on stdout, and how it ended.
#[derive(Debug)]
pub struct Captured {
    /// How it ended.
    pub status: ExitStatus,
    /// Its stdout, at most the number of bytes asked for.
    pub stdout: Vec<u8>,
    /// Whether it printed more than that.
    pub is_cut: bool,
}

/// Runs `command` with no input and stderr discarded, reads at most `max_stdout_bytes` of its
/// stdout while it runs, so that it never waits on a full pipe, and gives it `timeout` from its
/// start to end and close its stdout, stopping it past that. A process it started that keeps its
/// stdout open is not waited for past `timeout` either: the thread reading the output is left to
/// end with that process.
///
/// # Errors
///
/// The program cannot start (`NotFound` when it is not installed), it or its stdout does not end
/// within `timeout` (`TimedOut`), or its output cannot be read.
pub fn capture(
    command: &mut Command,
    timeout: Duration,
    max_stdout_bytes: u64,
) -> io::Result<Captured> {
    let deadline = Instant::now() + timeout;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("the program's output is not piped"))?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || drop(sender.send(read_capped(stdout, max_stdout_bytes))));
    let status = wait(&mut child, deadline, timeout)?;
    let read = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|error| match error {
            RecvTimeoutError::Timeout => timed_out("the program's output did not end", timeout),
            RecvTimeoutError::Disconnected => {
                io::Error::other("reading the program's output stopped unexpectedly")
            }
        })?;
    let (stdout, is_cut) = read?;
    Ok(Captured {
        status,
        stdout,
        is_cut,
    })
}

/// Runs `command` with `input` on its stdin and its output discarded, and returns how it ended and
/// how long it took from just before it started to the moment it ended, as an agent sees a hook
/// take. A thread of its own writes the input and waits for the program, so the time is not
/// rounded to a polling period, and a program that neither reads its input nor ends is given up on
/// at `timeout` all the same.
///
/// # Errors
///
/// The program cannot start, or cannot be given its input (it is then stopped), or does not end
/// within `timeout` (`TimedOut`): it is then left to end on its own, as a hook does by its own
/// deadline.
pub fn time_fed(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
) -> io::Result<(ExitStatus, Duration)> {
    let start = Instant::now();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let input = input.to_vec();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let ended = feed(&mut child, &input).and_then(|()| child.wait());
        drop(sender.send(ended.map(|status| (status, start.elapsed()))));
    });
    receiver
        .recv_timeout(timeout)
        .unwrap_or_else(|_| Err(timed_out("the program did not end", timeout)))
}

/// Writes `input` to `child`'s stdin and closes it. A program that ends before reading all of its
/// input is no error; any other failure to write stops the program.
fn feed(child: &mut Child, input: &[u8]) -> io::Result<()> {
    let Some(mut stdin) = child.stdin.take() else {
        return Ok(());
    };
    match stdin.write_all(input) {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            drop(child.kill());
            drop(child.wait());
            Err(error)
        }
        _ => Ok(()),
    }
}

/// Reads at most `max_bytes` of `output` and whether there was more, then the rest to its end, so
/// that the program can finish writing and exit.
fn read_capped(output: impl Read, max_bytes: u64) -> io::Result<(Vec<u8>, bool)> {
    let mut kept = Vec::new();
    let mut limited = output.take(max_bytes.saturating_add(1));
    limited.read_to_end(&mut kept)?;
    let is_cut = u64::try_from(kept.len()).is_ok_and(|len| len > max_bytes);
    kept.truncate(usize::try_from(max_bytes).unwrap_or(usize::MAX));
    io::copy(&mut limited.into_inner(), &mut io::sink())?;
    Ok((kept, is_cut))
}

/// Waits for `child` to exit until `deadline`, `timeout` after its start, and stops it past that.
fn wait(child: &mut Child, deadline: Instant, timeout: Duration) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            // The program may exit between the check and the kill; either way it is not waited
            // for again.
            drop(child.kill());
            drop(child.wait());
            return Err(timed_out("the program did not end", timeout));
        }
        std::thread::sleep(POLL_EVERY);
    }
}

/// A `TimedOut` error: `what` within `timeout`.
fn timed_out(what: &str, timeout: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{what} within {} s", timeout.as_secs()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command that runs for about `seconds`, reading no input, on this platform.
    fn sleeping(seconds: u32) -> Command {
        if cfg!(windows) {
            // `ping` waits a second between its echoes: n + 1 echoes take n seconds.
            let mut command = Command::new("ping");
            command.args(["-n", &(seconds + 1).to_string(), "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg(seconds.to_string());
            command
        }
    }

    /// `git --version` prints `git version` and a number: four bytes of it are kept and the output
    /// marked as cut; given room, the whole line comes, not cut.
    #[test]
    fn output_past_its_cap_is_cut() -> io::Result<()> {
        let short = capture(
            Command::new("git").arg("--version"),
            Duration::from_secs(10),
            4,
        )?;
        assert!(short.status.success());
        assert_eq!(short.stdout, b"git ");
        assert!(short.is_cut);
        let whole = capture(
            Command::new("git").arg("--version"),
            Duration::from_secs(10),
            4_096,
        )?;
        assert!(whole.stdout.starts_with(b"git version "));
        assert!(!whole.is_cut);
        Ok(())
    }

    /// A program that is not installed does not start, and says so.
    #[test]
    fn a_missing_program_is_not_found() {
        let missing = capture(
            &mut Command::new("cortex-test-no-such-program"),
            Duration::from_secs(5),
            16,
        );
        assert_eq!(
            missing.err().map(|error| error.kind()),
            Some(io::ErrorKind::NotFound)
        );
    }

    /// A program still running at its timeout is given up on, captured or fed, well before the
    /// 5 s it would take.
    #[test]
    fn programs_past_their_timeout_are_given_up_on() {
        let started = Instant::now();
        let captured = capture(&mut sleeping(5), Duration::from_millis(300), 16);
        assert_eq!(
            captured.err().map(|error| error.kind()),
            Some(io::ErrorKind::TimedOut)
        );
        let fed = time_fed(&mut sleeping(5), b"input", Duration::from_millis(300));
        assert_eq!(
            fed.err().map(|error| error.kind()),
            Some(io::ErrorKind::TimedOut)
        );
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
    }

    /// A program that ends without reading its input, more than a pipe holds, is timed to its end:
    /// the pipe it broke is no error.
    #[test]
    fn a_program_that_reads_no_input_is_timed_all_the_same() -> io::Result<()> {
        let input = vec![b'x'; 1 << 20];
        let (status, took) = time_fed(
            Command::new("git").arg("--version"),
            &input,
            Duration::from_secs(10),
        )?;
        assert!(status.success());
        assert!(took < Duration::from_secs(10), "{took:?}");
        Ok(())
    }

    /// A program that ends while a process it started keeps its stdout open is not waited for past
    /// the timeout: the shell here ends at once, and its `sleep` holds the output for 5 s.
    // A shell's background job is the plain way to make such a process on Unix; Windows has no
    // shell that does it as simply, and the code under test is the same on both.
    #[cfg(unix)]
    #[test]
    fn output_held_open_past_the_end_is_given_up_on() {
        let started = Instant::now();
        let captured = capture(
            Command::new("sh").args(["-c", "sleep 5 & echo started"]),
            Duration::from_millis(500),
            64,
        );
        assert_eq!(
            captured.err().map(|error| error.kind()),
            Some(io::ErrorKind::TimedOut)
        );
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
    }
}
