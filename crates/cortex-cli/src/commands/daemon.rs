//! `cortex daemon`: runs the process that keeps a repository's index and is its only writer (see
//! `cortex_daemon`), printing its notes on stderr. With `--relaunch`, which only a start on Windows
//! passes, it starts the daemon anew, holding none of this process's handles, and returns (see
//! `cortex_hook::relaunch_daemon`). With `--adopt`, which a hook passes for a folder not set up
//! yet once adoption is on, it first sets the folder up when every rule allows it, and otherwise
//! remembers the refusal and returns (see `init::adopt`).

use std::path::PathBuf;

use anyhow::Context;
use cortex_hook::Purpose;

use crate::commands::init::{Adopted, adopt};
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
    /// Set the folder up first when Cortex may set it up on its own: what a hook starts for a
    /// folder not set up yet.
    #[arg(long = "adopt", hide = true)]
    adopt: bool,
}

/// Runs the daemon for the repository until it has been idle long enough, once the folder is set
/// up when `--adopt` asks for it, and not at all when a rule refuses it, setting it up fails, or
/// another start is setting it up. The daemon names the repository by its canonical root itself.
///
/// # Errors
///
/// The current folder cannot be read, a refusal or failure cannot be written down, and the
/// daemon's own errors;
/// with `--relaunch`, the daemon could not be started, or the platform is not Windows.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = match &args.root {
        Some(root) => root.clone(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    let purpose = if args.adopt {
        Purpose::Adopt
    } else {
        Purpose::Serve
    };
    if args.relaunch {
        return relaunch(&root, purpose);
    }
    // The hold of a set-up is kept while the daemon serves the folder, so that another start for it
    // sets nothing up meanwhile.
    let (root, _hold) = match purpose {
        Purpose::Serve => (root, None),
        Purpose::Adopt => match adopt(&root)? {
            Adopted::Now(root, hold) | Adopted::Already(root, hold) => (root, Some(hold)),
            Adopted::Refused | Adopted::Failed | Adopted::Busy => return Ok(()),
        },
    };
    cortex_daemon::run(&root, &mut |line| output::note(line))
}

/// Starts the daemon for `root` and `purpose` anew with this binary and none of this process's
/// handles.
///
/// # Errors
///
/// This binary's path cannot be read, or the daemon could not be started.
#[cfg(windows)]
fn relaunch(root: &std::path::Path, purpose: Purpose) -> anyhow::Result<()> {
    let program = std::env::current_exe().context("reading this program's path")?;
    cortex_hook::relaunch_daemon(&program, root, purpose, std::env::home_dir().as_deref())
        .with_context(|| format!("starting the daemon for {} anew", root.display()))
}

/// Refuses: only a start on Windows asks for a relaunch.
///
/// # Errors
///
/// Always.
#[cfg(not(windows))]
fn relaunch(_root: &std::path::Path, _purpose: Purpose) -> anyhow::Result<()> {
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

    /// The flag a hook passes to set a folder up is the one `cortex daemon` reads.
    #[test]
    fn the_hook_s_flag_asks_for_an_adoption() -> Result<(), clap::Error> {
        let asked = Probe::try_parse_from(["daemon", cortex_hook::ADOPT_FLAG])?;
        assert!(asked.args.adopt);
        assert!(!Probe::try_parse_from(["daemon"])?.args.adopt);
        Ok(())
    }
}
