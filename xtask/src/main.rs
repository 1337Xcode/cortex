//! `cargo xtask`: the one entry point for every check in this repository.
//!
//! `cargo xtask verify` runs, in order: formatting, the architecture rules, the invariant tags,
//! clippy, the tests (doc tests included) and `cargo deny`. It stops at the first failure.
//! `cargo xtask verify --full` adds the budget checks run at milestone gates.

// xtask is a developer tool whose whole job is to report progress on the terminal.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod architecture;
mod invariants;
mod steps;

use std::process::ExitCode;

/// What `cargo xtask` was asked to do.
#[derive(Debug, PartialEq, Eq)]
enum Task {
    /// Run every check; `full` adds the budget checks.
    Verify {
        /// Whether to run the slow budget checks as well.
        full: bool,
    },
}

/// Parses the arguments after `cargo xtask`.
fn parse_task(args: &[String]) -> Result<Task, String> {
    match args {
        [command] if command == "verify" => Ok(Task::Verify { full: false }),
        [command, flag] if command == "verify" && flag == "--full" => {
            Ok(Task::Verify { full: true })
        }
        _ => Err(format!(
            "unknown arguments {args:?}; usage: cargo xtask verify [--full]"
        )),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = match parse_task(&args) {
        Ok(task) => task,
        Err(message) => {
            eprintln!("xtask: {message}");
            return ExitCode::from(2);
        }
    };
    let result = match task {
        Task::Verify { full } => steps::verify(full),
    };
    match result {
        Ok(()) => {
            println!("xtask: all checks passed");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask: FAILED: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turns string literals into the owned arguments `parse_task` receives.
    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn parses_verify_and_full() {
        assert_eq!(
            parse_task(&args(&["verify"])),
            Ok(Task::Verify { full: false })
        );
        assert_eq!(
            parse_task(&args(&["verify", "--full"])),
            Ok(Task::Verify { full: true })
        );
    }

    #[test]
    fn rejects_anything_else() {
        assert!(parse_task(&args(&[])).is_err());
        assert!(parse_task(&args(&["verify", "--fast"])).is_err());
        assert!(parse_task(&args(&["build"])).is_err());
    }
}
