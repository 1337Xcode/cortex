//! `cortex init`: registers Cortex's hooks with Claude Code, in the repository's
//! `.claude/settings.local.json` (`--scope project`, the default, a file Claude Code does not
//! commit) or in the user's `~/.claude/settings.json` (`--scope user`). The repository is the top
//! of the git repository holding the folder init starts from (see [`root`]). Nothing is written
//! when Claude Code is older than the oldest version Cortex's hooks work with. The file is backed
//! up before it changes, with a manifest, in the repository's `.cortex/backups/` or, for the
//! user's file, in `~/.cortex/backups/`, and written atomically; one that already holds Cortex's
//! current hooks is left as it is. Nothing of a repository's is written outside it: its `.claude`
//! and `.cortex` folders must be its own, not links elsewhere, and a settings file that is a link
//! must lead inside it.

mod backup;
mod root;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, bail};
use cortex_core::{UserFolder, canonical_root, create_private_folder, unix_millis};

use crate::commands::doctor::{Finding, claude_code_version};
use crate::{claude_settings, output};

use self::backup::Backup;

/// The arguments of `cortex init`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
    /// Where Claude Code reads the hooks from.
    #[arg(long, value_enum, default_value_t = Scope::Project)]
    scope: Scope,
}

/// Where Claude Code reads the hooks Cortex writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Scope {
    /// This repository only: `.claude/settings.local.json`.
    Project,
    /// Every project of the user: `~/.claude/settings.json`.
    User,
}

/// Where the settings file is and where its backup goes.
struct Target {
    /// The settings file, links followed.
    settings: PathBuf,
    /// The repository's root, for a project's file.
    root: Option<PathBuf>,
}

/// Registers Cortex's hooks with Claude Code at the scope asked for, and says where.
///
/// # Errors
///
/// A root that is not a folder, a settings file that leads outside the repository, is a link to
/// nothing, is read-only, too large, not UTF-8 or not the JSON Claude Code reads, or a backup or
/// a write that fails.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let adapter = cortex_hook::adapter("claude").context("finding the Claude Code adapter")?;
    let root = checked_root(args, adapter.versions())?;
    let target = target(args.scope, root)?;
    let existing = read_settings(&target.settings)?;
    let program = crate::program::this_program()?;
    let merged = claude_settings::merged(existing.as_deref(), &program, adapter.hooks())
        .with_context(|| {
            format!(
                "reading the Claude Code settings {}",
                target.settings.display()
            )
        })?;
    if existing.as_deref() == Some(merged.as_str()) {
        return output::line(&format!(
            "Claude Code: the hooks in {} are current",
            target.settings.display()
        ));
    }
    let backups = match &target.root {
        Some(root) => own_folder(&own_folder(root, ".cortex")?, "backups")?,
        None => {
            let user = UserFolder::of_current_user().context("finding the home folder")?;
            create_private_folder(user.path())
                .with_context(|| format!("creating {}", user.path().display()))?;
            own_folder(user.path(), "backups")?
        }
    };
    let mut backup = Backup::new(&backups, unix_millis(SystemTime::now()));
    let before = existing.as_deref().map(str::as_bytes);
    backup.replace(&target.settings, before, merged.as_bytes())?;
    output::line(&format!(
        "Claude Code: hooks written to {} (backup and manifest in {})",
        target.settings.display(),
        backup.folder().display()
    ))
}

/// The folder init sets up from where `args` start it: for a project's scope, the repository
/// [`root::find`] gives, said on stdout with why it is not git's top when it is not; for the
/// user's, the start folder, which only bounds where Claude Code may be found. Claude Code, asked
/// its version, must be one the `versions` Cortex knows allow.
///
/// # Errors
///
/// The folder or the home folder cannot be found, the folder would be the home folder or hold it,
/// Claude Code is older than the oldest of `versions`, or stdout cannot be written.
fn checked_root(args: &Args, versions: cortex_hook::Versions) -> anyhow::Result<PathBuf> {
    let start = match &args.root {
        Some(root) => root.clone(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    let folder = match args.scope {
        Scope::Project => {
            let home = std::env::home_dir().context("finding the home folder")?;
            let found = root::find(&start, &home)?;
            output::line(&format!("repository: {}", found.folder.display()))?;
            if let Some(why) = &found.without_git {
                output::line(&format!(
                    "note: {why}; Cortex sets up this folder alone, without git history"
                ))?;
            }
            found.folder
        }
        Scope::User => canonical_root(&start)
            .with_context(|| format!("finding the folder {}", start.display()))?,
    };
    match claude_code_version(&folder, versions) {
        Finding::Fail { what, fix } => bail!("{what}; {fix}, then run `cortex init` again"),
        Finding::Pass(what) => output::line(&what)?,
        Finding::Note(what) => output::line(&format!("note: {what}"))?,
    }
    Ok(folder)
}

/// The settings file `scope` names for the repository at `root`, links followed, and the
/// repository it belongs to for a project's.
///
/// # Errors
///
/// No home folder for the user's scope, a `.claude` folder that is not the repository's own, or a
/// settings file that is a link to nothing or, for a project, leads outside the repository.
fn target(scope: Scope, root: PathBuf) -> anyhow::Result<Target> {
    match scope {
        Scope::Project => {
            let claude = own_folder(&root, ".claude")?;
            let settings = followed(&claude.join("settings.local.json"))?;
            if !settings.starts_with(&root) {
                bail!(
                    "{} leads outside the repository, to {}; Cortex does not write there",
                    claude.join("settings.local.json").display(),
                    settings.display()
                );
            }
            Ok(Target {
                settings,
                root: Some(root),
            })
        }
        Scope::User => {
            let home = std::env::home_dir().context("finding the home folder")?;
            let claude = home.join(".claude");
            fs::create_dir_all(&claude)
                .with_context(|| format!("creating {}", claude.display()))?;
            Ok(Target {
                settings: followed(&claude.join("settings.json"))?,
                root: None,
            })
        }
    }
}

/// The folder `name` inside `parent`, made when missing, which must be a folder of its own there:
/// one that is a link leads elsewhere, and is refused before anything is written through it.
///
/// # Errors
///
/// The folder cannot be made or found, or it leads elsewhere.
fn own_folder(parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let folder = parent.join(name);
    if let Err(error) = fs::symlink_metadata(&folder) {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error).with_context(|| format!("finding {}", folder.display()));
        }
        create_private_folder(&folder).with_context(|| format!("creating {}", folder.display()))?;
    }
    let real = canonical_root(&folder).with_context(|| format!("finding {}", folder.display()))?;
    // The parents are compared, not the names: on Windows a folder's name may be spelled in
    // another case than asked for, and a link to a folder beside it stays inside all the same.
    if real.parent() != Some(canonical_root(parent)?.as_path()) {
        bail!(
            "{} leads elsewhere, to {}; Cortex does not write there",
            folder.display(),
            real.display()
        );
    }
    Ok(real)
}

/// The file at `path`, followed through a link to the file it leads to; `path` itself, in its
/// folder's canonical form, when nothing is there.
///
/// # Errors
///
/// A link that leads to nothing (writing would replace it, losing where it led), or a path that
/// cannot be followed.
fn followed(path: &Path) -> anyhow::Result<PathBuf> {
    let target = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() && !path.exists() => {
            bail!(
                "{} is a link to a file that does not exist; Cortex leaves it as it is",
                path.display()
            );
        }
        Ok(_) => fs::canonicalize(path).with_context(|| format!("finding {}", path.display()))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error).with_context(|| format!("finding {}", path.display())),
    };
    let (parent, file) = target
        .parent()
        .zip(target.file_name())
        .with_context(|| format!("{} names no file", target.display()))?;
    Ok(canonical_root(parent)
        .with_context(|| format!("finding {}", parent.display()))?
        .join(file))
}

/// What the settings file at `path` holds, or `None` when there is none.
///
/// # Errors
///
/// The file is read-only (the user keeps it as it is), or cannot be read as
/// [`claude_settings::read`] reads it.
fn read_settings(path: &Path) -> anyhow::Result<Option<String>> {
    if fs::metadata(path).is_ok_and(|metadata| metadata.permissions().readonly()) {
        bail!("{} is read-only; Cortex leaves it as it is", path.display());
    }
    claude_settings::read(path)
}
