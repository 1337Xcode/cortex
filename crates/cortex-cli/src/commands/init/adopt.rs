//! Setting a repository up on its own: what `cortex daemon --adopt` does before it serves the
//! folder a hook offered it, once the user turned adoption on (`cortex init --global`).
//!
//! One start sets a folder up at a time: the start takes a lock of the folder's own in the user's
//! run folder, and keeps it while it then serves the folder; a second start finds it taken and
//! ends having written nothing. The hook looked at file metadata alone; here every rule is checked
//! again with the whole configuration: the folder must still be a git working tree's top Cortex
//! may set up on its own (below the home folder, not the user's own Cortex folder, not the
//! temporary folder, no `off` at or above it: [`cortex_core::adoption_candidate`]), no pattern of
//! `adopt.exclude` may match it, git must name it its working tree's top, and in a linked working
//! tree the same rules hold for its repository's main tree, which is the one set up. A folder a
//! rule refuses is remembered (see [`crate::adoption`]), so that hooks stop offering it. A folder
//! already set up on this machine is served as it is.
//!
//! Then it is set up as `cortex init --yes` sets a repository up, with no agent's hooks (those are
//! the user's own, written by `--global`): its ignore lines go to the exclude file git keeps for
//! it, never to a `.gitignore` its team commits, each change backed up so that `cortex uninstall`
//! undoes it, and `.cortex/adopted` tells the daemon to tell the user once. All or nothing: when
//! anything fails once writing began, every file is given back what it held and everything made is
//! removed, so that the folder is as it was. A failure (a configuration that cannot be read, git
//! that does not answer, a file that cannot be written) is logged and not remembered: the next
//! hook offers the folder again.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, anyhow};
use cortex_core::{
    ADOPTED_FILE, AdoptConfig, Places, RepoId, UserFolder, adoption_candidate, canonical_root,
    create_private_folder, open_private_file,
};

use super::root::Root;
use super::{Files, Scope, apply, consent, place, root};
use crate::adoption;
use crate::backup::replace_unchanged;

/// What became of a folder offered.
#[derive(Debug)]
pub(crate) enum Adopted {
    /// It is set up now, as the repository whose root is this, for the daemon to serve while the
    /// hold is kept.
    Now(PathBuf, Hold),
    /// It was set up on this machine already, as the repository whose root is this.
    Already(PathBuf, Hold),
    /// A rule refused it, and the refusal is remembered.
    Refused,
    /// Setting it up failed; the failure is logged, and the folder is as it was.
    Failed,
    /// Another start is setting it up or serving it; nothing was done.
    Busy,
}

/// The lock that keeps a second start for a folder from setting it up, held while the daemon
/// started for it runs.
#[derive(Debug)]
pub(crate) struct Hold(#[allow(dead_code, reason = "held for its lock, which ends with it")] File);

/// Why a folder is not set up.
enum NotSetUp {
    /// A rule refuses it, for this reason: remembered.
    Rule(String),
    /// Setting it up failed: logged.
    Failure(anyhow::Error),
}

impl From<anyhow::Error> for NotSetUp {
    fn from(error: anyhow::Error) -> Self {
        Self::Failure(error)
    }
}

/// Sets the folder `folder` up on its own when every rule allows it, as the module says.
///
/// # Errors
///
/// No home folder, a folder that cannot be found, a lock that cannot be taken for another reason
/// than another start holding it, or a refusal or failure that cannot be written down.
pub(crate) fn adopt(folder: &Path) -> anyhow::Result<Adopted> {
    let home = std::env::home_dir().context("finding the home folder")?;
    let user = UserFolder::in_home(&home);
    let folder = canonical_root(folder).with_context(|| format!("finding {}", folder.display()))?;
    let Some(hold) = held(&user, &folder)? else {
        return Ok(Adopted::Busy);
    };
    let decided = decide(&folder, &home).and_then(|plan| match plan {
        Plan::Already(root) => Ok(Adopted::Already(root, hold)),
        Plan::SetUp(root) => Ok(Adopted::Now(set_up(root)?, hold)),
    });
    match decided {
        Ok(adopted) => Ok(adopted),
        Err(NotSetUp::Rule(why)) => {
            adoption::remember(&user, &folder, &why)?;
            Ok(Adopted::Refused)
        }
        Err(NotSetUp::Failure(error)) => {
            adoption::log_failure(&user, &folder, &error, SystemTime::now())?;
            Ok(Adopted::Failed)
        }
    }
}

/// What to do with a folder every rule allows.
enum Plan {
    /// Serve it: the repository whose root is this is set up on this machine already.
    Already(PathBuf),
    /// Set it up.
    SetUp(Root),
}

/// The lock of `folder`'s set-up in the user's folder `user`, taken; none when another start holds
/// it.
///
/// # Errors
///
/// The lock's file cannot be made, or the lock cannot be taken for another reason.
fn held(user: &UserFolder, folder: &Path) -> anyhow::Result<Option<Hold>> {
    let path = user.adopting_path(&RepoId::of(folder));
    if let Some(run) = path.parent() {
        create_private_folder(run).with_context(|| format!("creating {}", run.display()))?;
    }
    let file = open_private_file(&path).with_context(|| format!("opening {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Hold(file))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("locking {}", path.display()))
        }
    }
}

/// What to do with `folder`, canonical, for the user whose home folder is `home`.
///
/// # Errors
///
/// A rule that refuses the folder, or a failure to tell.
fn decide(folder: &Path, home: &Path) -> Result<Plan, NotSetUp> {
    let places = Places::of(home, &std::env::temp_dir()).ok_or_else(|| {
        anyhow!("the home folder cannot be found, is a relative path or is the file system's root")
    })?;
    // The rules that need no configuration first: a folder they refuse has none of its own read.
    placed(folder, &places)?;
    if let Some(located) = cortex_hook::locate(folder, Some(home)) {
        return Ok(Plan::Already(located.repository_root().to_path_buf()));
    }
    let adopt = cortex_daemon::settings::load(folder)
        .context("its configuration cannot be read")?
        .config
        .adopt;
    not_excluded(folder, &places, &adopt)?;
    let root = root::find(folder, home)?;
    if let Some(why) = &root.without_git {
        return Err(NotSetUp::Failure(anyhow!("{why}")));
    }
    let top = root.linked.as_deref().unwrap_or(&root.folder);
    if top != folder {
        return Err(NotSetUp::Rule(format!(
            "git names {} as the top of its working tree",
            top.display()
        )));
    }
    if root.folder != folder {
        placed(&root.folder, &places)
            .and_then(|()| not_excluded(&root.folder, &places, &adopt))
            .map_err(|refusal| match refusal {
                NotSetUp::Rule(why) => NotSetUp::Rule(format!(
                    "its repository's main working tree {}: {why}",
                    root.folder.display()
                )),
                failure @ NotSetUp::Failure(_) => failure,
            })?;
    }
    Ok(Plan::SetUp(root))
}

/// Refuses `folder`, canonical, when it is no git working tree's top Cortex may set up on its own,
/// kept away from `places`.
///
/// # Errors
///
/// The rule that refuses it.
fn placed(folder: &Path, places: &Places) -> Result<(), NotSetUp> {
    let candidate = adoption_candidate(folder, places)
        .map_err(|refusal| NotSetUp::Rule(refusal.said().to_owned()))?;
    if candidate.folder != folder {
        return Err(NotSetUp::Rule(
            "it is not the top of the git working tree it is in".to_owned(),
        ));
    }
    Ok(())
}

/// Refuses `folder` when a pattern of `adopt.exclude` in `adopt` matches it, `~` being the home
/// folder of `places`.
///
/// # Errors
///
/// The pattern that matches.
fn not_excluded(folder: &Path, places: &Places, adopt: &AdoptConfig) -> Result<(), NotSetUp> {
    match adopt.excluding(folder, &places.home) {
        Some(pattern) => Err(NotSetUp::Rule(format!("`adopt.exclude` holds `{pattern}`"))),
        None => Ok(()),
    }
}

/// Sets the repository `root` up, all or nothing, and returns its root folder.
///
/// # Errors
///
/// A file that cannot be read or written: everything written is undone first.
fn set_up(mut root: Root) -> anyhow::Result<PathBuf> {
    root.is_adopted = true;
    let files = Files::found(Scope::Project, &root, &[], false)?;
    let before = Before::of(&root, &files)?;
    let kept = files.kept()?;
    let answers = consent::answers(kept, true, None::<&mut io::Empty>)?;
    let changes = files.changes(kept, answers)?;
    let written = apply((&root.folder, root.linked.as_deref()), changes).and_then(|()| {
        let cortex = place::own_folder(&root.folder, ".cortex")?;
        let adopted = cortex.join(ADOPTED_FILE);
        open_private_file(&adopted).with_context(|| format!("writing {}", adopted.display()))?;
        Ok(())
    });
    if let Err(error) = written {
        let restored = before.restore();
        return Err(match restored {
            Ok(()) => error,
            Err(undoing) => {
                error.context(format!("and undoing what was written failed: {undoing:#}"))
            }
        });
    }
    Ok(root.folder)
}

/// What the files a set-up may write held before it wrote any, and which folders were there.
struct Before {
    /// The repository's `.cortex` folder, and whether it was there.
    cortex: (PathBuf, bool),
    /// The names in its `backups` folder; none when the folder was not there.
    repository_backups: Option<BTreeSet<OsString>>,
    /// The files a set-up changes in place, each with what it held (`None`: no file): the exclude
    /// file, and the repository's configuration.
    files: Vec<(PathBuf, Option<String>)>,
    /// The user's backup folder, and the names in it.
    user_backups: (PathBuf, BTreeSet<OsString>),
}

impl Before {
    /// What `files`, found for `root`, held, and which folders were there.
    ///
    /// # Errors
    ///
    /// No home folder, or the user's backup folder cannot be read.
    fn of(root: &Root, files: &Files) -> anyhow::Result<Self> {
        let cortex = root.folder.join(".cortex");
        let user = UserFolder::of_current_user().context("finding the home folder")?;
        let backups = user.path().join("backups");
        let mut changed = vec![(
            files.repository_config.file.clone(),
            files.repository_config.text.clone(),
        )];
        if let Some((_, ignore)) = &files.ignores {
            changed.push((ignore.file.clone(), ignore.text.clone()));
        }
        let repository_backups = cortex
            .join("backups")
            .exists()
            .then(|| names(&cortex.join("backups")))
            .transpose()?;
        Ok(Self {
            repository_backups,
            cortex: (cortex.clone(), cortex.exists()),
            files: changed,
            user_backups: (backups.clone(), names(&backups)?),
        })
    }

    /// Gives each file back what it held and removes what this process made since.
    ///
    /// # Errors
    ///
    /// A file cannot be read, written or removed.
    fn restore(&self) -> anyhow::Result<()> {
        for (file, text) in &self.files {
            given_back(file, text.as_deref())?;
        }
        let (cortex, had_cortex) = &self.cortex;
        if !had_cortex {
            removed(cortex)?;
        } else {
            match &self.repository_backups {
                Some(there) => removed_runs(&cortex.join("backups"), there)?,
                None => removed(&cortex.join("backups"))?,
            }
            removed(&cortex.join(ADOPTED_FILE))?;
        }
        let (backups, there) = &self.user_backups;
        removed_runs(backups, there)
    }
}

/// The names in `folder`; none when it is not there.
///
/// # Errors
///
/// The folder cannot be read.
fn names(folder: &Path) -> anyhow::Result<BTreeSet<OsString>> {
    match fs::read_dir(folder) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<_>>()
            .with_context(|| format!("reading {}", folder.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", folder.display())),
    }
}

/// Gives `file` back `text`, what it held, or removes it when it held nothing; a file that holds
/// it already is left as it is.
///
/// # Errors
///
/// The file cannot be read, written or removed.
fn given_back(file: &Path, text: Option<&str>) -> anyhow::Result<()> {
    let now = match fs::read(file) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("reading {}", file.display())),
    };
    match (now, text) {
        (Some(now), Some(text)) if now != text.as_bytes() => {
            replace_unchanged(file, Some(&now), text.as_bytes())
        }
        (Some(_), None) => removed(file),
        _ => Ok(()),
    }
}

/// Removes the file or the folder at `path`, with all it holds, when it is there.
///
/// # Errors
///
/// It is there and cannot be removed.
fn removed(path: &Path) -> anyhow::Result<()> {
    let removing = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match removing {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

/// Removes, from the backup folder `backups`, each run folder this process made (named for its
/// process id, `<time>-<id>`) that is not among `there`, the names it held before.
///
/// # Errors
///
/// The folder cannot be read, or a run folder cannot be removed.
fn removed_runs(backups: &Path, there: &BTreeSet<OsString>) -> anyhow::Result<()> {
    let ours = format!("-{}", std::process::id());
    for name in names(backups)? {
        let is_ours = name.to_str().is_some_and(|name| name.ends_with(&ours));
        if is_ours && !there.contains(&name) {
            removed(&backups.join(&name))?;
        }
    }
    Ok(())
}
