//! The files of Cortex's own an agent reads from its folder (see [`crate::agents::own`]): for a
//! project's scope, in the repository's folder the agent reads, each file one git is told to leave
//! out; for the user's scope, or an agent with no folder of the repository's, in the user's. A file
//! already at a path that is not Cortex's is the user's, and init refuses to write over it (a file
//! an older build wrote is told by its marker, its folder's, or a manifest's record of it). An
//! agent that loads both folders would run Cortex twice, so init refuses one folder while the other
//! holds Cortex's files; and a first file in a repository's folder that would hide a file of the
//! user's from the agent (Roo Code's `.roorules`) is refused too. An agent whose user's folder
//! Cortex does not write gets none of the files at the user's scope, and init says what to do
//! instead.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{IGNORE_LOCAL_FILES, MAX_SETTINGS_BYTES, UserFolder, read_text_file};

use super::agent_file::{AgentFile, LeftOut, What};
use super::{Owner, Scope, place};
use crate::agents::Setup;
use crate::agents::own::{
    OwnFile, OwnFiles, OwnKind, Signs, UsersFolder, holds_cortex_files, is_cortex_s,
    is_in_marked_folder,
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
/// `program`, and what init says instead when it writes none for the user; none for an agent
/// Cortex writes no such files for. Nothing is written.
///
/// # Errors
///
/// No home folder, or the system does not name the user's folder when it is needed; a file that
/// leads outside the repository, is a link to nothing, is read-only, cannot be read, or is the
/// user's own; the other scope's folder holds Cortex's files; a first file would hide one of the
/// user's; or the files cannot be made.
pub(super) fn planned(
    scope: Scope,
    root: Option<&Path>,
    setup: &'static Setup,
    program: &str,
) -> anyhow::Result<(Vec<AgentFile>, Option<&'static str>)> {
    let Some(own) = &setup.own else {
        return Ok((Vec::new(), None));
    };
    let written = (own.files)(program)?;
    let folder = match (scope, own.project, root, own.user) {
        (Scope::Project, Some(project), Some(root), _) => {
            refuse_hiding((root, project), own, setup, &written)?;
            Folder::Repository { root, project }
        }
        (.., UsersFolder::Told(told)) => return Ok((Vec::new(), Some(told))),
        (.., UsersFolder::At(user)) => Folder::User(user.resolved().with_context(|| {
            format!(
                "finding the folder {} reads the user's {}s from; set it up with `--scope \
                 project` in a repository instead",
                setup.agent.title(),
                own.kind.word()
            )
        })?),
    };
    if own.loads_both
        && let (Some(project), Some(root)) = (own.project, root)
    {
        refuse_both_scopes((root, project), &folder, setup, &written)?;
    }
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let recorded = runs::own_files_written(&user)?;
    let files = written
        .iter()
        .map(|file| planned_file(setup, (&folder, &recorded, own.kind), file, &written))
        .collect::<anyhow::Result<_>>()?;
    Ok((files, None))
}

/// Refuses when a file of the repository's at `root` that the agent reads only while its folder
/// `project` holds no file (`own.hides`) exists, and that folder holds none: a first file of
/// Cortex's there, one of `written`, would hide it. The folder is checked to be the repository's
/// own before anything in it is read. A file of Cortex's there already is not refused again: it
/// hides that file already, and uninstall takes it out.
///
/// # Errors
///
/// The folder leads outside the repository or cannot be read, or Cortex's file would hide one of
/// the user's.
fn refuse_hiding(
    (root, project): (&Path, &str),
    own: &OwnFiles,
    setup: &Setup,
    written: &[OwnFile],
) -> anyhow::Result<()> {
    let Some(hidden) = own
        .hides
        .iter()
        .map(|file| place::joined(root, file))
        .find(|file| file.is_file())
    else {
        return Ok(());
    };
    let paths = written
        .iter()
        .map(|file| place::file_in(root, Some(project), &file.name))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let folder = place::joined(root, project);
    if paths.iter().any(|path| path.exists()) || holds_files(&folder)? {
        return Ok(());
    }
    let title = setup.agent.title();
    bail!(
        "{title} reads {} only while {} holds no file, so Cortex's {} there would hide it; put a \
         file of yours in that folder ({title} reads every file there, and the rules of {} can \
         move into it), then run init again",
        hidden.display(),
        folder.display(),
        own.kind.word(),
        hidden.display()
    )
}

/// Whether `folder` holds a file, in it or in a folder of it, a link counting as a file; false
/// for a folder that does not exist.
///
/// # Errors
///
/// The folder or one in it cannot be read.
fn holds_files(folder: &Path) -> anyhow::Result<bool> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", folder.display()));
        }
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        // A link is not followed: it may lead out of the repository.
        let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if !is_folder || holds_files(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The file `file` of `setup`'s agent, of `kind`, planned in `folder`, one of `written`, given
/// what runs of init `recorded` writing, by path.
///
/// # Errors
///
/// As [`planned`], for this file.
fn planned_file(
    setup: &'static Setup,
    (folder, recorded, kind): (&Folder<'_>, &BTreeMap<PathBuf, Vec<String>>, OwnKind),
    file: &OwnFile,
    written: &[OwnFile],
) -> anyhow::Result<AgentFile> {
    let (path, owner, left_out) = placed(folder, &file.name, kind)?;
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
        what: What::Own(kind.word()),
        left_out,
        is_executable: file.is_executable,
    })
}

/// Where the file `name` of `kind`, with `/` between folders, goes in `folder`, whose it is, and
/// for a file of the repository's how git is told to leave it out: a rules file by the exclude
/// file, any other by a line of the `.gitignore` block.
///
/// # Errors
///
/// A file of the repository's in a folder of `folder`'s, or one git is told nothing about; a file
/// that leads outside the repository or is a link to nothing.
fn placed(
    folder: &Folder<'_>,
    name: &str,
    kind: OwnKind,
) -> anyhow::Result<(PathBuf, Owner, Option<LeftOut>)> {
    match folder {
        Folder::Repository { root, project } => {
            // A folder of the repository's is made with its record only as deep as `project`.
            if name.contains('/') {
                bail!("{name} is not a file of {project} itself");
            }
            let relative = format!("{project}/{name}");
            let left_out = if kind == OwnKind::Rules {
                LeftOut::Exclude(relative)
            } else {
                IGNORE_LOCAL_FILES
                    .iter()
                    .copied()
                    .find(|line| *line == relative)
                    .map(LeftOut::Gitignore)
                    .with_context(|| format!("git is told nothing to leave {relative} out"))?
            };
            let path = place::file_in(root, Some(project), name)?;
            let owner = Owner::Repository {
                folder: Some(project),
            };
            Ok((path, owner, Some(left_out)))
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
        Folder::Repository { .. } => match own.user.path().map(|user| user.resolved()) {
            Some(Ok(users)) => (users, " --scope user"),
            Some(Err(_)) | None => return Ok(()),
        },
    };
    if holds_cortex_files(&other, written)? {
        let title = setup.agent.title();
        let what = own.kind.word();
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
