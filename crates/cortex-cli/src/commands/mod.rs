//! The command registry.
//!
//! Every command is one module in this folder plus one variant of [`Command`] and one arm in
//! [`run`]. Removing a command means deleting all three.

mod version;

use clap::Subcommand;

/// Every command `cortex` accepts.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Print the version of this binary.
    Version,
}

/// Runs one parsed command to completion.
///
/// # Errors
///
/// Returns the command's own error, already carrying the context of what failed.
pub(crate) fn run(command: Command) -> anyhow::Result<()> {
    match command {
        Command::Version => version::run(),
    }
}
