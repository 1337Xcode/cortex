//! The files of Cortex's own an agent reads from its folder (see [`crate::agents::own`]): for a
//! project's scope, in the repository's folder the agent reads, each file one git is told to leave
//! out; for the user's scope, or an agent with no folder of the repository's, in the user's. A file
//! already at a path that is not Cortex's is the user's, and init refuses to write over it (a file
//! an older build wrote is told by its marker, its folder's, or a manifest's record of it). An
//! agent that loads both folders would run Cortex twice, so init refuses one folder while the other
//! holds Cortex's files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{IGNORE_LOCAL_FILES, MAX_SETTINGS_BYTES, UserFolder, read_text_file};

use super::agent_file::{AgentFile, What};
use super::{Owner, Scope, place};
use crate::agents::Setup;
use crate::agents::own::{
    OwnFile, OwnFiles, Signs, holds_cortex_files, is_cortex_s, is_in_marked_folder,
};
use crate::backup::Kind;
use crate::commands::uninstall::runs;

/// The folder the files go to.
enum Folder<'r> {
    /// The repository's folder `project`, relative to its root `root`.
    Repository {
        /// The repository's canonical root.
        root: &'r Path,
        /// The folder, relative to the root.
        project: &'static str,
    },
    /// The user's folder.
    User(PathBuf),
}

/// The files of Cortex's own `scope` names for `setup`'s agent, for the repository whose canonical
/// root is `root` (none for the user's files alone, as `--global` writes them), each running
/// `program`; none for an agent Cortex writes no such files for. Nothing is written.
///
/// # Errors
///
/// No home folder, or no folder of the user's on this system when the user's is needed; a file
/// that leads outside the repository, is a link to nothing, is read-only, cannot be read, or is
/// the user's own; the other scope's folder holds Cortex's files; or the files cannot be made.
pub(super) fn planned(
    scope: Scope,
    root: Option<&Path>,
    setup: &'static Setup,
    program: &str,
) -> anyhow::Result<Vec<AgentFile>> {
    let Some(own) = &setup.own else {
        return Ok(Vec::new());
    };
    let written = (own.files)(program)?;
    let folder = match (scope, own.project, root) {
        (Scope::Project, Some(project), Some(root)) => Folder::Repository { root, project },
        _ => Folder::User(users_folder(setup, own)?),
    };
    if own.loads_both
        && let (Some(project), Some(root)) = (own.project, root)
    {
        refuse_both_scopes((root, project), &folder, setup, &written)?;
    }
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let recorded = runs::own_files_written(&user)?;
    written
        .iter()
        .map(|file| planned_file(setup, (&folder, &recorded), file, &written))
        .collect()
}

/// The user's folder `setup`'s agent reads files of Cortex's own from.
///
/// # Errors
///
/// No home folder, the agent documents none on this system, or the system does not name it.
fn users_folder(setup: &Setup, own: &OwnFiles) -> anyhow::Result<PathBuf> {
    let title = setup.agent.title();
    let Some(user) = own.user else {
        bail!(
            "{title} documents no folder of the user's for its {}s on this system; set it up with \
             `--scope project` in a repository",
            own.what
        );
    };
    user.resolved().with_context(|| {
        format!(
            "finding the folder {title} reads the user's {}s from; set it up with `--scope \
             project` in a repository instead",
            own.what
        )
    })
}

/// The file `file` of `setup`'s agent planned in `folder`, one of `written`, given what runs of
/// init `recorded` writing, by path.
///
/// # Errors
///
/// As [`planned`], for this file.
fn planned_file(
    setup: &'static Setup,
    (folder, recorded): (&Folder<'_>, &BTreeMap<PathBuf, Vec<String>>),
    file: &OwnFile,
    written: &[OwnFile],
) -> anyhow::Result<AgentFile> {
    let (path, owner, ignore_line) = placed(folder, &file.name)?;
    place::writable(&path)?;
    let before = read_text_file(&path, MAX_SETTINGS_BYTES)?;
    let agents_folder = match folder {
        Folder::Repository { root, project } => place::joined(root, project),
        Folder::User(folder) => folder.clone(),
    };
    let signs = Signs {
        recorded: recorded.get(&path).map_or(&[], Vec::as_slice),
        is_in_marked_folder: is_in_marked_folder(&agents_folder, file, written),
    };
    if before
        .as_deref()
        .is_some_and(|before| !is_cortex_s(before, file, signs))
    {
        bail!(
            "{} is a file of yours, not Cortex's, and Cortex does not write over it; move it \
             elsewhere, then run init again",
            path.display()
        );
    }
    Ok(AgentFile {
        setup,
        file: path,
        before,
        after: file.text.clone(),
        owner,
        kind: Kind::AgentFile {
            agent: setup.agent.name().to_owned(),
        },
        what: What::Own(setup.own.as_ref().map_or("file", |own| own.what)),
        ignore_line,
        is_executable: file.is_executable,
    })
}

/// Where the file `name`, with `/` between folders, goes in `folder`, whose it is, and for a file
/// of the repository's the line that has git leave it out.
///
/// # Errors
///
/// A file of the repository's in a folder of `folder`'s, or one git is told nothing about; a file
/// that leads outside the repository or is a link to nothing.
fn placed(
    folder: &Folder<'_>,
    name: &str,
) -> anyhow::Result<(PathBuf, Owner, Option<&'static str>)> {
    match folder {
        Folder::Repository { root, project } => {
            // A folder of the repository's is made with its record only as deep as `project`.
            if name.contains('/') {
                bail!("{name} is not a file of {project} itself");
            }
            let relative = format!("{project}/{name}");
            let line = IGNORE_LOCAL_FILES
                .iter()
                .copied()
                .find(|line| *line == relative)
                .with_context(|| format!("git is told nothing to leave {relative} out"))?;
            let path = place::file_in(root, Some(project), name)?;
            let owner = Owner::Repository {
                folder: Some(project),
            };
            Ok((path, owner, Some(line)))
        }
        Folder::User(folder) => {
            let (inner, name) = name.rsplit_once('/').unwrap_or(("", name));
            let parent = if inner.is_empty() {
                folder.clone()
            } else {
                place::joined(folder, inner)
            };
            Ok((place::file_beside(&parent, name)?, Owner::User, None))
        }
    }
}

/// Refuses when the other scope's folder, `project` in the repository at `root` when `folder` is
/// the user's and the user's folder otherwise, holds one of `written` as Cortex's: the agent loads
/// both folders, so each event would run Cortex twice.
///
/// # Errors
///
/// No home folder, a file that cannot be read, or one of Cortex's there.
fn refuse_both_scopes(
    (root, project): (&Path, &str),
    folder: &Folder<'_>,
    setup: &Setup,
    written: &[OwnFile],
) -> anyhow::Result<()> {
    let Some(own) = &setup.own else {
        return Ok(());
    };
    let (other, flag) = match folder {
        Folder::User(_) => (place::joined(root, project), ""),
        // A folder the system does not name is one the agent, asking the system the same way,
        // cannot find and load either.
        Folder::Repository { .. } => match own.user.map(|user| user.resolved()) {
            Some(Ok(users)) => (users, " --scope user"),
            Some(Err(_)) | None => return Ok(()),
        },
    };
    if holds_cortex_files(&other, written)? {
        let title = setup.agent.title();
        let what = own.what;
        bail!(
            "{} holds Cortex's {title} {what} already, and {title} loads the {what}s of both \
             folders, so each event would run Cortex twice; take it out first (`cortex \
             uninstall{flag}` undoes what init wrote there), or keep it and set {title} up in \
             that scope",
            other.display()
        );
    }
    Ok(())
}
