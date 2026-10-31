//! `cortex daemon`: runs the process that keeps a repository's index and is its only writer (see
//! `cortex_daemon`), printing its notes on stderr.

use std::path::PathBuf;

use anyhow::Context;

use crate::output;

/// The arguments of `cortex daemon`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Runs the daemon for the repository until it has been idle long enough. The daemon names the
/// repository by its canonical root itself.
///
/// # Errors
///
/// The current folder cannot be read, and the daemon's own errors.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = match &args.root {
        Some(root) => root.clone(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    cortex_daemon::run(&root, &mut |line| output::note(line))
}
