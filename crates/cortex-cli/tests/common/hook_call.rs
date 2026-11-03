//! Running `cortex hook` as an agent does: a payload on its standard input, its output and how
//! long it took kept for the test to judge.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::{TestResult, isolate};

/// How a hook call went: its output and how long it took.
pub struct Call {
    /// What the process left behind.
    pub output: Output,
    /// From its start to its exit.
    pub took: Duration,
    /// Whether the whole input could be written: not when the hook had stopped reading and closed
    /// its end first.
    pub is_input_taken: bool,
}

impl Call {
    /// Whether the call ended as an agent requires: exit code 0, nothing printed.
    pub fn is_silent_success(&self) -> bool {
        self.output.status.code() == Some(0)
            && self.output.stdout.is_empty()
            && self.output.stderr.is_empty()
    }
}

/// Runs `cortex hook` with `args`, writing `input` to its standard input after `delay` from a
/// thread of its own (a hook that stops reading may close the pipe first), with `home` as the
/// user's home folder.
pub fn hook(args: &[&str], input: Vec<u8>, delay: Duration, home: &Path) -> TestResult<Call> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .arg("hook")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate(&mut command, home);
    let started = Instant::now();
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().ok_or("stdin is not piped")?;
    let writer = thread::spawn(move || {
        thread::sleep(delay);
        // The hook may have stopped reading and closed its end: that is its right.
        stdin.write_all(&input).is_ok()
    });
    let output = child.wait_with_output()?;
    let took = started.elapsed();
    let is_input_taken = writer.join().map_err(|_| "the writer panicked")?;
    Ok(Call {
        output,
        took,
        is_input_taken,
    })
}
