//! The `cortex` binary.
//!
//! Parses the command line, runs the chosen command and turns any error into a one-line message on
//! stderr with exit code 1. Each command lives in its own module under `commands`.

mod commands;
mod output;

use std::process::ExitCode;

use clap::Parser;

use crate::commands::Command;

/// Command-line interface of `cortex`.
#[derive(Debug, Parser)]
#[command(
    name = "cortex",
    version,
    about = "A local memory and map of a codebase for coding agents."
)]
struct Cli {
    /// The command to run.
    #[command(subcommand)]
    command: Command,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match commands::run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::error(&error);
            ExitCode::FAILURE
        }
    }
}
