//! The `cortex` binary.
//!
//! Parses the command line, runs the chosen command and turns any error into a one-line message on
//! stderr with exit code 1. Each command lives in its own module under `commands`.

mod backup;
mod claude_settings;
mod commands;
mod output;
mod program;
mod repository;
mod upgrade;

use std::process::ExitCode;

use clap::Parser;

use crate::commands::Command;

/// Command-line interface of `cortex`.
#[derive(Debug, Parser)]
#[command(
    name = "cortex",
    // Help and usage name the binary `cortex`, not its file name (`cortex.exe` on Windows).
    bin_name = "cortex",
    version,
    about = "A local memory and map of a codebase for coding agents."
)]
struct Cli {
    /// The command to run.
    #[command(subcommand)]
    command: Command,
}

/// Runs the command named on the command line; exit code 1 and a message on stderr on failure.
/// `cortex hook` runs before the command line is parsed: whatever its arguments, it exits 0, as an
/// agent requires of a hook, and prints nothing but the agent's hook output when the daemon gave
/// it text in time.
fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|command| command == "hook") {
        let rest: Vec<_> = args.collect();
        let home = std::env::home_dir();
        cortex_hook::run(
            &rest,
            std::io::stdin(),
            home.as_deref(),
            &mut std::io::stdout(),
        );
        // INVARIANT: fail-open. The hook ends with exit code 0 whatever happened inside it.
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse();
    match commands::run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::error(&error);
            ExitCode::FAILURE
        }
    }
}
