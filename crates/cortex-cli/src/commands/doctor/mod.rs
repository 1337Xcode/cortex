//! `cortex doctor`: whether Claude Code will run Cortex's hooks for this repository and the daemon
//! will answer them, and whether every other agent init set up for it still holds Cortex's hooks,
//! one check at a time, each failure with what to do about it, and what it cannot see.
//!
//! The checks are a table, [`CHECKS`]: adding one means one function and one line. Each reads the
//! settings files Claude Code reads that Cortex may read (the repository's local and shared
//! settings, the user's, and the managed ones), the hooks files init's manifests say it wrote for
//! other agents, asks the daemon, asks Claude Code its version, or runs this binary's hook on
//! probes; none runs a command a settings file names, since a repository can name any program
//! there.

mod adoption;
mod agent;
mod agents;
mod daemon;
mod hooks;
mod latency;
mod settings;

use std::path::PathBuf;

use anyhow::bail;

use crate::{output, repository};

use self::settings::SettingsFile;

pub(crate) use self::agent::version as claude_code_version;

/// The arguments of `cortex doctor`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// What the checks look at.
#[derive(Debug)]
pub(super) struct Context {
    /// The repository's root folder.
    root: PathBuf,
    /// This binary, as the hooks would name it.
    program: String,
    /// The settings files, highest precedence first.
    files: Vec<SettingsFile>,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Finding {
    /// All is well.
    Pass(String),
    /// Something is wrong, and what to do about it.
    Fail {
        /// What is wrong.
        what: String,
        /// What to do.
        fix: String,
    },
}

/// A check: its name, and the function that makes its findings.
type Check = (&'static str, fn(&Context) -> Vec<Finding>);

/// Every check, in the order they run and print.
const CHECKS: &[Check] = &[
    ("hooks", hooks::registered),
    ("silenced", hooks::silenced),
    ("binary", hooks::binary),
    ("agents", agents::current),
    ("agent", agent::check),
    ("daemon", daemon::answers),
    ("latency", latency::probes),
    ("adoption", adoption::state),
];

/// Runs every check on the repository, prints each finding, and fails when any check did.
///
/// # Errors
///
/// A root that is not a folder, this binary's path not found or not UTF-8, stdout that cannot be
/// written, or a check that failed.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let failed = checked(root)?;
    if failed > 0 {
        bail!("{failed} of the checks failed");
    }
    Ok(())
}

/// Runs every check on the repository whose canonical root is `root`, prints each finding, and
/// returns how many checks failed.
///
/// # Errors
///
/// This binary's path is not found or not UTF-8, or stdout cannot be written.
pub(crate) fn checked(root: PathBuf) -> anyhow::Result<usize> {
    let context = Context {
        program: crate::program::this_program()?,
        files: settings::files(&root),
        root,
    };
    let mut failed = 0;
    for (name, check) in CHECKS {
        for finding in check(&context) {
            match finding {
                Finding::Pass(what) => output::line(&format!("ok    {name}: {what}"))?,
                Finding::Fail { what, fix } => {
                    failed += 1;
                    output::line(&format!("FAIL  {name}: {what}"))?;
                    output::line(&format!("      fix: {fix}"))?;
                }
            }
        }
    }
    Ok(failed)
}
