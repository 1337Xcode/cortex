//! `cortex daemon`: runs the process that keeps a repository's index and is its only writer (see
//! `cortex_daemon`), printing its notes on stderr.

use std::path::PathBuf;

use crate::{output, repository};

/// The arguments of `cortex daemon`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Runs the daemon for the repository until it has been idle long enough.
///
/// # Errors
///
/// A root that is not a folder, and the daemon's own errors.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    cortex_daemon::run(&root, &mut |line| output::note(line))
}
