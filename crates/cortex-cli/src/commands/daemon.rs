//! `cortex daemon`: runs the process that keeps a repository's index and is its only writer (see
//! `cortex_daemon`), printing its notes on stderr. With `--relaunch`, which only a start on Windows
//! passes, it starts the daemon anew, holding none of this process's handles, and returns (see
//! `cortex_hook::relaunch_daemon`).

use std::path::PathBuf;

use anyhow::Context;

use crate::output;

/// The arguments of `cortex daemon`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
    /// Start the daemon anew, inheriting nothing from this process, instead of running it: what a
    /// start on Windows runs.
    #[arg(long = "relaunch", hide = true)]
    relaunch: bool,
}

/// Runs the daemon for the repository until it has been idle long enough. The daemon names the
/// repository by its canonical root itself.
///
/// # Errors
///
/// The current folder cannot be read, and the daemon's own errors; with `--relaunch`, the daemon
/// could not be started, or the platform is not Windows.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = match &args.root {
        Some(root) => root.clone(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    if args.relaunch {
        return relaunch(&root);
    }
    cortex_daemon::run(&root, &mut |line| output::note(line))
}

/// Starts the daemon for `root` anew with this binary and none of this process's handles.
///
/// # Errors
///
/// This binary's path cannot be read, or the daemon could not be started.
#[cfg(windows)]
fn relaunch(root: &std::path::Path) -> anyhow::Result<()> {
    let program = std::env::current_exe().context("reading this program's path")?;
    cortex_hook::relaunch_daemon(&program, root, std::env::home_dir().as_deref())
        .with_context(|| format!("starting the daemon for {} anew", root.display()))
}

/// Refuses: only a start on Windows asks for a relaunch.
///
/// # Errors
///
/// Always.
#[cfg(not(windows))]
fn relaunch(_root: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!(
        "`cortex daemon {}` is only for Windows",
        cortex_hook::RELAUNCH_FLAG
    )
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    /// `cortex daemon` as the CLI parses it.
    #[derive(Debug, Parser)]
    struct Probe {
        /// The command's arguments.
        #[command(flatten)]
        args: Args,
    }

    /// The flag a start passes is the one `cortex daemon` reads, and plain `cortex daemon` runs the
    /// daemon.
    #[test]
    fn the_start_s_flag_asks_for_a_relaunch() -> Result<(), clap::Error> {
        let asked = Probe::try_parse_from(["daemon", cortex_hook::RELAUNCH_FLAG])?;
        assert!(asked.args.relaunch);
        assert!(!Probe::try_parse_from(["daemon"])?.args.relaunch);
        Ok(())
    }
}
