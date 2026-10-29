//! Where the CLI's own commands write to the terminal. Argument parsing writes help, the version
//! flag and usage errors itself.
//!
//! Writes go through locked handles and report failures instead of panicking, so a closed pipe
//! (`cortex version | head -0`) ends the command with an error rather than a crash.

use std::io::{self, Write};

use anyhow::Context;

/// Writes one line of normal output to stdout.
///
/// # Errors
///
/// Fails when stdout cannot be written.
pub(crate) fn line(text: &str) -> anyhow::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{text}").context("writing to stdout")
}

/// Writes a failed command's error, with its chain of causes, to stderr as one line.
///
/// A failure to write the message itself is ignored: there is nowhere left to report it.
pub(crate) fn error(error: &anyhow::Error) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "cortex: error: {error:#}");
}
