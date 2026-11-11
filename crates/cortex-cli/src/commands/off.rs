//! `cortex off`: turns Cortex off in a repository. It writes `.cortex/off`, so that hooks there,
//! and in every folder below it, give nothing and start no daemon, and Cortex never sets the
//! repository up on its own; then it stops the repository's daemon. What Cortex keeps of the
//! repository stays, for `cortex on` to turn it on again. A `.cortex` folder it makes for the
//! marker gets a `.gitignore` of its own that has git leave the folder out, so that turning Cortex
//! off in a repository it never set up changes nothing git shows.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::Context as _;
use cortex_core::{OFF_FILE, UserFolder, open_private_file};

use super::init::own_folder;
use super::uninstall::stopped;
use crate::{output, repository};

/// What the `.gitignore` of a `.cortex` folder `cortex off` makes holds: git leaves out the folder
/// and all it holds.
pub(crate) const OFF_GITIGNORE: &str =
    "# Written by `cortex off`: git leaves this folder out.\n*\n";

/// The arguments of `cortex off`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the repository, or the git working tree, the current folder
    /// is in when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Turns Cortex off in the repository `args` name, as the module says, and says so.
///
/// # Errors
///
/// No home folder, a folder that cannot be found or is the home folder, a marker that cannot be
/// written, a daemon that does not stop, or an output that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::switched_root(args.root.as_deref())?;
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let is_new = !root.join(".cortex").exists();
    let cortex = own_folder(&root, ".cortex")?;
    if is_new {
        let ignore = cortex.join(".gitignore");
        fs::write(&ignore, OFF_GITIGNORE)
            .with_context(|| format!("writing {}", ignore.display()))?;
    }
    let off = cortex.join(OFF_FILE);
    let mut marker =
        open_private_file(&off).with_context(|| format!("writing {}", off.display()))?;
    marker
        .set_len(0)
        .and_then(|()| writeln!(marker, "Cortex is off here; `cortex on` turns it on again."))
        .with_context(|| format!("writing {}", off.display()))?;
    // The marker first: a hook that comes while the daemon stops starts none again.
    let stopped = stopped(&root, &user)?;
    output::line(&format!(
        "cortex: off in {}; `cortex on` turns it on again",
        root.display()
    ))?;
    output::line(&stopped)
}
