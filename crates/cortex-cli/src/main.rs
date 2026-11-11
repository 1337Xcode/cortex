//! The `cortex` binary.
//!
//! Parses the command line, runs the chosen command and turns any error into a one-line message on
//! stderr with exit code 1. Each command lives in its own module under `commands`.

mod adoption;
mod agents;
mod backup;
mod commands;
mod output;
mod program;
mod repository;
mod shared_facts;
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
/// it text in time. With no arguments and a standard input that is not a terminal, it prints
/// nothing and exits 0 (see [`is_hook_without_arguments`]).
fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.is_none() && is_hook_without_arguments() {
        // INVARIANT: fail-open. An agent that runs Claude Code's exec-form hook without its
        // arguments must not read clap's usage error (exit 2) as a refusal of its tool call.
        return ExitCode::SUCCESS;
    }
    if first.is_some_and(|command| command == "hook") {
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
    run(cli)
}

/// Whether `cortex`, run with no arguments, is run as a hook: its standard input is a pipe or a
/// file rather than a terminal. Cortex writes Claude Code's hooks as a program with an argument
/// list; an agent that imports Claude Code's settings and runs the program alone would get clap's
/// usage error and exit code 2, which several agents take as "block". A person at a terminal still
/// gets the usage.
fn is_hook_without_arguments() -> bool {
    use std::io::IsTerminal as _;
    !std::io::stdin().is_terminal()
}

/// Runs the parsed command; exit code 1 and a message on stderr on failure.
fn run(cli: Cli) -> ExitCode {
    match commands::run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::error(&error);
            ExitCode::FAILURE
        }
    }
}
