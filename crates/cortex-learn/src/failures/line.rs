//! What a failed command line's output says failed: the output read by the parser of the check the
//! line's exit is the failure of, else by that of each check the line ran. Agents write
//! `cd x && pytest` and `pytest | tail`, whose exit is not pytest's alone, though what they print
//! is pytest's.

use std::path::Path;

use cortex_core::ShellCommand;

use super::{Failures, parse_failures};
use crate::commands::Commands;

/// A failed line's output, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineFailure {
    /// What the output says failed.
    pub failures: Failures,
    /// The runners of the line's checks, each once: the failing check's first, then the others in
    /// the order of the line.
    pub runners: Vec<&'static str>,
}

/// `output`, what the line `command`, run in the folder `cwd` of the repository whose top is
/// `root`, printed when it failed, read as `commands` tells the line's checks; none when the line
/// ran no check of a runner with a parser, or no parser finds a failure in it.
#[must_use]
pub fn read_line_failure(
    commands: &Commands,
    command: &ShellCommand,
    (root, cwd): (&Path, &Path),
    output: &str,
) -> Option<LineFailure> {
    let told = commands.tell(command, root, cwd);
    let mut runners: Vec<&'static str> = Vec::new();
    for check in told.failure.iter().chain(&told.checks) {
        if !runners.contains(&check.runner) {
            runners.push(check.runner);
        }
    }
    let failures = runners
        .iter()
        .find_map(|runner| parse_failures(runner, output))?;
    Some(LineFailure { failures, runners })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed `cd x && python -m pytest -q`, whose exit may be the `cd`'s, is read by pytest's
    /// parser: one failed of three, the failing test named. A line of no check reads nothing.
    #[test]
    fn a_failed_line_is_read_by_its_checks_parsers() -> Result<(), Box<dyn std::error::Error>> {
        let commands = Commands::new(&[])?;
        let output = "FAILED tests/test_ops.py::test_divide - ZeroDivisionError\n\
                      1 failed, 2 passed in 0.03s\n";
        let root = Path::new("/repo");
        let line = ShellCommand::cut("cd /repo && python -m pytest -q");
        let read = read_line_failure(&commands, &line, (root, root), output).ok_or("unread")?;
        assert_eq!(read.runners, ["pytest"]);
        assert_eq!(
            read.failures
                .counts
                .map(|counts| (counts.failed, counts.passed)),
            Some((1, Some(2)))
        );
        assert_eq!(
            read.failures.failed_tests,
            ["tests/test_ops.py::test_divide"]
        );
        let other = ShellCommand::cut("ls -la");
        assert_eq!(
            read_line_failure(&commands, &other, (root, root), output),
            None
        );
        Ok(())
    }
}
