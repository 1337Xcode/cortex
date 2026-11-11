//! `cortex on`: turns Cortex on again in a repository `cortex off` turned it off in. It removes
//! `.cortex/off`, and the refusal remembered for the folder, if Cortex refused to set it up on its
//! own; a `.cortex` folder that `cortex off` made and that holds nothing else then goes too, so
//! that the repository is again one Cortex may set up on its own. The next hook there starts the
//! daemon.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cortex_core::{OFF_FILE, UserFolder};

use super::off::OFF_GITIGNORE;
use crate::{output, repository};

/// The arguments of `cortex on`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the repository, or the git working tree, the current folder
    /// is in when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Turns Cortex on again in the repository `args` name, as the module says, and says what it did.
///
/// # Errors
///
/// No home folder, a folder that cannot be found or is the home folder, a file that cannot be
/// removed, or an output that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::switched_root(args.root.as_deref())?;
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let cortex = root.join(".cortex");
    let was_off = removed(&cortex.join(OFF_FILE))?;
    let was_refused = crate::adoption::forget(&user, &root)?;
    if was_off {
        forget_off_folder(&cortex)?;
    }
    let said = match (was_off, was_refused) {
        (false, false) => format!("cortex: was not off in {}", root.display()),
        _ => format!("cortex: on in {}", root.display()),
    };
    output::line(&said)
}

/// Removes the file at `path`, and says whether it was there.
///
/// # Errors
///
/// It is there and cannot be removed.
fn removed(path: &Path) -> anyhow::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

/// Removes the `.cortex` folder `cortex` when it holds nothing but the `.gitignore` `cortex off`
/// wrote there, unchanged: the folder was made for the marker alone.
///
/// # Errors
///
/// The folder cannot be read, or the file or the folder cannot be removed.
fn forget_off_folder(cortex: &Path) -> anyhow::Result<()> {
    let names: Vec<_> = fs::read_dir(cortex)
        .with_context(|| format!("reading {}", cortex.display()))?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<_>>()
        .with_context(|| format!("reading {}", cortex.display()))?;
    let ignore = cortex.join(".gitignore");
    let is_offs_alone = names == [".gitignore"]
        && fs::read(&ignore).is_ok_and(|text| text == OFF_GITIGNORE.as_bytes());
    if is_offs_alone {
        fs::remove_file(&ignore).with_context(|| format!("removing {}", ignore.display()))?;
        fs::remove_dir(cortex).with_context(|| format!("removing {}", cortex.display()))?;
    }
    Ok(())
}
