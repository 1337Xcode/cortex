//! The command registry.
//!
//! Every command is one module in this folder plus one variant of [`Command`] and one arm in
//! [`run`]. Removing a command means deleting all three.

mod claims;
mod consolidate;
mod daemon;
mod doctor;
mod export;
mod facts;
mod import;
mod init;
mod mcp;
mod note;
mod receipts;
mod status;
mod uninstall;
mod version;
mod where_;
mod why;

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
    /// Set Cortex up in a repository for the coding agents found on this machine.
    Init(init::Args),
    /// Check that Claude Code will run Cortex's hooks and the daemon will answer them.
    Doctor(doctor::Args),
    /// Undo what init did to a repository and stop its daemon.
    Uninstall(uninstall::Args),
    /// Show what each task of recent sessions took: turns, tool calls, tokens and estimated cost.
    Receipts(receipts::Args),
    /// Show the code the retrieval engine finds for a question, with its reasons and what the
    /// gate would say.
    Where(where_::Args),
    /// Show why code is where it is: its links, the rules about it, who works on it now and its
    /// open warnings.
    Why(why::Args),
    /// Serve Cortex's MCP tools on stdio (started by the agent).
    Mcp(mcp::Args),
    /// Note a project rule about files or symbols, given to agents working on them.
    Note(note::Args),
    /// List the project rules, or confirm, retire or correct one.
    Facts(facts::Args),
    /// List advisory claims on files, claim files for the agent session at work, or release
    /// claims.
    Claims(claims::Args),
    /// Ask the daemon to run upkeep now.
    Consolidate(consolidate::Args),
    /// Write the active project rules to .cortex/shared/ for a team that commits it.
    Export(export::Args),
    /// Take a team's shared project rules from .cortex/shared/, each to confirm before it is used.
    Import(import::Args),
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
        Command::Where(args) => where_::run(&args),
        Command::Why(args) => why::run(&args),
        Command::Mcp(args) => mcp::run(&args),
        Command::Note(args) => note::run(&args),
        Command::Facts(args) => facts::run(&args),
        Command::Claims(args) => claims::run(&args),
        Command::Consolidate(args) => consolidate::run(&args),
        Command::Export(args) => export::run(&args),
        Command::Import(args) => import::run(&args),
    }
}
