//! `cortex doctor`: whether Claude Code will run Cortex's hooks for this repository and the daemon
//! will answer them, one check at a time, each failure with what to do about it, and what it
//! cannot see.
//!
//! The checks are a table, [`CHECKS`]: adding one means one function and one line. Each reads the
//! settings files Claude Code reads that Cortex may read (the repository's local and shared
//! settings, the user's, and the managed ones) or asks the daemon; none runs a command a settings
//! file names, since a repository can name any program there.

mod daemon;
mod hooks;
mod settings;

use std::path::PathBuf;

use anyhow::bail;

use crate::{output, repository};

use self::settings::SettingsFile;

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
pub(super) enum Finding {
    /// All is well.
    Pass(String),
    /// Something is wrong, and what to do about it.
    Fail {
        /// What is wrong.
        what: String,
        /// What to do.
        fix: String,
    },
    /// Something worth knowing that is not wrong.
    Note(String),
}

/// A check: its name, and the function that makes its findings.
type Check = (&'static str, fn(&Context) -> Vec<Finding>);

/// Every check, in the order they run and print.
const CHECKS: &[Check] = &[
    ("hooks", hooks::registered),
    ("silenced", hooks::silenced),
    ("binary", hooks::binary),
    ("daemon", daemon::answers),
];

/// What no check can see, said after them all.
const UNSEEN: &str = "doctor cannot see whether Claude Code runs with --safe-mode, with --settings \
     that turn hooks off, under settings an organization delivers by its server, MDM or registry, \
     or in a folder it does not trust; each of these keeps every hook from running";

/// Runs every check on the repository, prints each finding, and fails when any check did.
///
/// # Errors
///
/// A root that is not a folder, this binary's path not found or not UTF-8, stdout that cannot be
/// written, or a check that failed.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
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
                Finding::Note(what) => output::line(&format!("note  {name}: {what}"))?,
            }
        }
    }
    output::line(&format!("note  {UNSEEN}"))?;
    if failed > 0 {
        bail!("{failed} of the checks failed");
    }
    Ok(())
}
