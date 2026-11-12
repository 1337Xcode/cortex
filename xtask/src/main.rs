//! `cargo xtask`: the one entry point for every check in this repository.
//!
//! `cargo xtask verify` runs, in order: formatting, the architecture rules, workspace-lint
//! inheritance, the invariant tags, clippy, the tests (doc tests included), `cargo deny`, the
//! plugins' checks and the map's checks. It stops at the first failure.
//! `cargo xtask verify --full` adds the slow tests, the map's browser checks and the budget checks.
//! `cargo xtask ui` installs the map's packages and builds `ui/dist` for the daemon to embed; run
//! it before a release build, or the binary serves a page saying it has no map.

// xtask is a developer tool whose whole job is to report progress on the terminal.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod architecture;
mod budgets;
mod cfg;
mod comments;
mod compiled;
mod invariants;
mod lints;
mod map_budgets;
mod steps;
mod ui;

use std::process::ExitCode;

/// What `cargo xtask` was asked to do.
#[derive(Debug, PartialEq, Eq)]
enum Task {
    /// Run every check; `full` adds the budget checks.
    Verify {
        /// Whether to run the slow budget checks as well.
        full: bool,
    },
    /// Install the map's packages and build it.
    Ui,
}

/// Parses the arguments after `cargo xtask`.
fn parse_task(args: &[String]) -> Result<Task, String> {
    match args {
        [command] if command == "verify" => Ok(Task::Verify { full: false }),
        [command, flag] if command == "verify" && flag == "--full" => {
            Ok(Task::Verify { full: true })
        }
        [command] if command == "ui" => Ok(Task::Ui),
        _ => Err(format!(
            "unknown arguments {args:?}; usage: cargo xtask verify [--full] | cargo xtask ui"
        )),
    }
}

/// Runs the requested task; exit code 2 on bad arguments, 1 when a check fails.
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
        Task::Verify { full } => steps::verify(full).map(|()| "all checks passed"),
        Task::Ui => steps::workspace_root()
            .and_then(|root| ui::install_and_build(&root))
            .map(|()| "the map is built in ui/dist"),
    };
    match result {
        Ok(done) => {
            println!("xtask: {done}");
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

    /// `verify`, `verify --full` and `ui` are the accepted forms.
    #[test]
    fn parses_verify_full_and_ui() {
        assert_eq!(
            parse_task(&args(&["verify"])),
            Ok(Task::Verify { full: false })
        );
        assert_eq!(
            parse_task(&args(&["verify", "--full"])),
            Ok(Task::Verify { full: true })
        );
        assert_eq!(parse_task(&args(&["ui"])), Ok(Task::Ui));
    }

    /// Every other argument list is a usage error.
    #[test]
    fn rejects_anything_else() {
        assert!(parse_task(&args(&[])).is_err());
        assert!(parse_task(&args(&["verify", "--fast"])).is_err());
        assert!(parse_task(&args(&["build"])).is_err());
        assert!(parse_task(&args(&["ui", "--full"])).is_err());
    }
}
