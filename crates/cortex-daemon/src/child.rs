//! Running another program the daemon needs an answer from (git, `whoami`), bounded in time and in
//! the output read.

use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How often a running program is checked on.
const POLL_EVERY: Duration = Duration::from_millis(10);

/// What a program printed on stdout, and how it ended.
#[derive(Debug)]
pub(crate) struct Captured {
    /// How it ended.
    pub(crate) status: ExitStatus,
    /// Its stdout, at most the number of bytes asked for.
    pub(crate) stdout: Vec<u8>,
    /// Whether it printed more than that.
    pub(crate) is_cut: bool,
}

/// Runs `command` with no input and stderr discarded, reads at most `max_stdout_bytes` of its
/// stdout while it runs, so that it never waits on a full pipe, and waits at most `timeout` for it
/// to end, stopping it past that.
///
/// # Errors
///
/// The program cannot start (`NotFound` when it is not installed), does not end within `timeout`
/// (`TimedOut`), or its output cannot be read.
pub(crate) fn capture(
    command: &mut Command,
    timeout: Duration,
    max_stdout_bytes: u64,
) -> io::Result<Captured> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("the program's output is not piped"))?;
    let reader = std::thread::spawn(move || -> io::Result<(Vec<u8>, bool)> {
        let mut output = Vec::new();
        let mut limited = stdout.take(max_stdout_bytes + 1);
        limited.read_to_end(&mut output)?;
        let is_cut = u64::try_from(output.len()).is_ok_and(|len| len > max_stdout_bytes);
        output.truncate(usize::try_from(max_stdout_bytes).unwrap_or(usize::MAX));
        // The rest is drained so that the program can finish writing and exit.
        io::copy(&mut limited.into_inner(), &mut io::sink())?;
        Ok((output, is_cut))
    });
    let status = wait(&mut child, timeout)?;
    let (stdout, is_cut) = reader
        .join()
        .map_err(|_| io::Error::other("reading the program's output stopped unexpectedly"))??;
    Ok(Captured {
        status,
        stdout,
        is_cut,
    })
}

/// Waits for `child` to exit, at most `timeout`, and stops it past that.
fn wait(child: &mut Child, timeout: Duration) -> io::Result<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() > timeout {
            // The program may exit between the check and the kill; either way it is not waited
            // for again.
            drop(child.kill());
            drop(child.wait());
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("the program did not end within {} s", timeout.as_secs()),
            ));
        }
        std::thread::sleep(POLL_EVERY);
    }
}
