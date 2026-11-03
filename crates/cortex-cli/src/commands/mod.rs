//! The command registry.
//!
//! Every command is one module in this folder plus one variant of [`Command`] and one arm in
//! [`run`]. Removing a command means deleting all three.

mod daemon;
mod doctor;
mod init;
mod receipts;
mod status;
mod uninstall;
mod version;

use clap::Subcommand;

/// Every command `cortex` accepts.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Print the version of this binary.
    Version,
    /// Run the process that keeps a repository's index (started by Cortex itself).
    Daemon(daemon::Args),
    /// Show whether the daemon runs, the index's state and the scale mode.
    Status(status::Args),
    /// Register Cortex's hooks with Claude Code.
    Init(init::Args),
    /// Check that Claude Code will run Cortex's hooks and the daemon will answer them.
    Doctor(doctor::Args),
    /// Undo what init did to a repository and stop its daemon.
    Uninstall(uninstall::Args),
    /// Show what each task of recent sessions took: turns, tool calls, tokens and estimated cost.
    Receipts(receipts::Args),
}

/// Runs one parsed command to completion.
///
/// # Errors
///
/// Returns the command's own error, already carrying the context of what failed.
pub(crate) fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Version => version::run(),
        Command::Daemon(args) => daemon::run(&args),
        Command::Status(args) => status::run(&args),
        Command::Init(args) => init::run(&args),
        Command::Doctor(args) => doctor::run(&args),
        Command::Uninstall(args) => uninstall::run(&args),
        Command::Receipts(args) => receipts::run(&args),
    }
}
