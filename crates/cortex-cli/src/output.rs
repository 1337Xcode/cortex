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

/// Writes one line about the process itself, such as a long-running command's progress or a
/// configuration warning, to stderr, after `cortex: `, its control characters escaped.
///
/// # Errors
///
/// Fails when stderr cannot be written.
pub(crate) fn note(text: &str) -> anyhow::Result<()> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "cortex: {}", printable(text)).context("writing to stderr")
}

/// Writes a failed command's error, with its chain of causes, to stderr as one line, its control
/// characters escaped.
///
/// A failure to write the message itself is ignored: there is nowhere left to report it.
pub(crate) fn error(error: &anyhow::Error) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(
        stderr,
        "cortex: error: {}",
        printable(&format!("{error:#}"))
    );
}

/// `text` with each control character written as its escape (`\u{1b}`, `\n`): text from a
/// repository, such as a configuration key in a warning or a path in an error, cannot move the
/// cursor, change colours or split the line.
fn printable(text: &str) -> String {
    let mut printable = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() {
            printable.extend(character.escape_default());
        } else {
            printable.push(character);
        }
    }
    printable
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Escape sequences, line breaks and other control characters come out as their escapes;
    /// everything else, accents included, as it was.
    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(
            printable("key \u{1b}[31mred\u{1b}[0m\nnext\tcafé"),
            "key \\u{1b}[31mred\\u{1b}[0m\\nnext\\tcafé"
        );
    }
}
