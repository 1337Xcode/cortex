//! Which repository `cortex mcp` serves for a call: one set up on this machine and not turned off,
//! found from the folder `--root` names, else from the first of the client's roots in one, else
//! from the current folder, read only then, since an editor may have deleted the folder it started
//! the server in. Never the home folder, a folder holding it, or a file system's root, where an
//! editor that starts the server from no repository in particular would have it run. When none is
//! served, the reason is a sentence the agent passes on to the user.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use cortex_core::{UserFolder, canonical_root, is_never_a_root, is_set_up_here, is_turned_off};

/// Why no repository is served for a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// No repository set up on this machine holds the folder.
    NotSetUp(PathBuf),
    /// The repository holding it, set up here, is turned off.
    TurnedOff(PathBuf),
    /// The folder, or the repository holding it, is the home folder, a folder holding it, or a
    /// file system's root.
    NeverARoot(PathBuf),
    /// The client named no root in a repository served, and the current folder cannot be read.
    NoFolder(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSetUp(folder) => write!(
                formatter,
                "Cortex is not set up for {}; tell the user that `cortex init` in the repository \
                 sets it up",
                folder.display()
            ),
            Self::TurnedOff(root) => write!(
                formatter,
                "Cortex is turned off in {}; tell the user that `cortex on` there turns it on",
                root.display()
            ),
            Self::NeverARoot(folder) => write!(
                formatter,
                "{} is the home folder, a folder holding it, or a file system's root, which Cortex \
                 never serves; tell the user to start the agent in a repository set up with \
                 `cortex init`",
                folder.display()
            ),
            Self::NoFolder(why) => write!(
                formatter,
                "the current folder cannot be read ({why}), and no root the client named is in a \
                 repository set up on this machine"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// The repository served, its canonical root and the top of the working tree paths are read
/// from, for the user whose home folder is `home`: found from `given`, else from the first of
/// `roots` it is found from, else from the folder `current` gives. The refusal said is the first
/// folder's.
///
/// # Errors
///
/// No repository is served from any of them.
pub(crate) fn served(
    given: Option<&Path>,
    roots: &[PathBuf],
    home: &Path,
    current: impl FnOnce() -> io::Result<PathBuf>,
) -> Result<(PathBuf, PathBuf), Refusal> {
    let home = canonical_root(home).unwrap_or_else(|_| home.to_path_buf());
    if let Some(given) = given {
        return served_from(given, &home);
    }
    let mut first = None;
    for root in roots {
        match served_from(root, &home) {
            Ok(served) => return Ok(served),
            Err(refusal) => {
                first.get_or_insert(refusal);
            }
        }
    }
    let refusal = match current() {
        Ok(folder) => match served_from(&folder, &home) {
            Ok(served) => return Ok(served),
            Err(refusal) => refusal,
        },
        Err(error) => Refusal::NoFolder(error.to_string()),
    };
    Err(first.unwrap_or(refusal))
}

/// The repository served from `folder` for the user whose canonical home folder is `home`, as
/// [`served`] gives it.
///
/// # Errors
///
/// The folder or its repository is never a root, its repository is turned off, or no repository
/// set up here holds it.
fn served_from(folder: &Path, home: &Path) -> Result<(PathBuf, PathBuf), Refusal> {
    let Ok(canonical) = canonical_root(folder) else {
        return Err(Refusal::NotSetUp(folder.to_path_buf()));
    };
    if is_never_a_root(&canonical, home) {
        return Err(Refusal::NeverARoot(canonical));
    }
    if let Some(located) = cortex_hook::locate(&canonical, Some(home)) {
        let root = located.repository_root().to_path_buf();
        // A folder outside the home folder may be in one that holds it.
        if is_never_a_root(&root, home) {
            return Err(Refusal::NeverARoot(root));
        }
        return Ok((root, located.root));
    }
    let user = UserFolder::in_home(home);
    match cortex_hook::locate_marked(&canonical, Some(home)) {
        Some(marked)
            if is_set_up_here(&user, marked.repository_root())
                && is_turned_off(marked.repository_root()) =>
        {
            Err(Refusal::TurnedOff(marked.repository_root().to_path_buf()))
        }
        _ => Err(Refusal::NotSetUp(canonical)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a test's result is.
    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A home folder and folders beside it, all under one temporary folder, canonical.
    struct Folders {
        /// The temporary folder holding everything, removed when dropped.
        _top: tempfile::TempDir,
        /// Its canonical path.
        top: PathBuf,
        /// The home folder, `top/home`.
        home: PathBuf,
    }

    /// A temporary folder holding an empty home folder.
    fn folders() -> TestResult<Folders> {
        let made = tempfile::tempdir()?;
        let top = canonical_root(made.path())?;
        let home = top.join("home");
        std::fs::create_dir(&home)?;
        Ok(Folders {
            _top: made,
            top,
            home,
        })
    }

    /// Makes `root` a repository set up for the user whose home folder is `home`, turned off when
    /// `is_off`, and gives its canonical path.
    fn set_up(root: &Path, home: &Path, is_off: bool) -> TestResult<PathBuf> {
        std::fs::create_dir_all(root.join(".cortex"))?;
        if is_off {
            std::fs::write(root.join(".cortex").join(cortex_core::OFF_FILE), "")?;
        }
        let root = canonical_root(root)?;
        cortex_core::record_set_up(&UserFolder::in_home(home), &root)?;
        Ok(root)
    }

    /// A current folder that cannot be read.
    fn unreadable() -> io::Result<PathBuf> {
        Err(io::Error::new(io::ErrorKind::NotFound, "deleted"))
    }

    /// Started in the home folder, the server serves the set-up repository the client names as a
    /// root, after a root that is in none; and from a folder in it, the repository's top.
    #[test]
    fn a_root_in_a_set_up_repository_is_served_from_the_home_folder() -> TestResult {
        let folders = folders()?;
        let repo = set_up(&folders.top.join("repo"), &folders.home, false)?;
        std::fs::create_dir(repo.join("src"))?;
        let elsewhere = folders.top.join("elsewhere");
        std::fs::create_dir(&elsewhere)?;
        let home = folders.home.clone();
        let roots = [elsewhere, repo.join("src")];
        let served = served(None, &roots, &folders.home, || Ok(home))?;
        assert_eq!(served, (repo.clone(), repo));
        Ok(())
    }

    /// The home folder itself, with no root named, is refused as a folder never served, and so
    /// is a folder holding it, named as a root, even set up; a folder outside the home folder
    /// inside such a set-up folder is refused for it.
    #[test]
    fn the_home_folder_and_a_folder_holding_it_are_never_served() -> TestResult {
        let folders = folders()?;
        let home = folders.home.clone();
        let refused = served(None, &[], &folders.home, || Ok(home.clone()));
        assert_eq!(refused, Err(Refusal::NeverARoot(home.clone())));
        let top = set_up(&folders.top, &folders.home, false)?;
        let refused = served(None, std::slice::from_ref(&top), &folders.home, || {
            Ok(home.clone())
        });
        assert_eq!(refused, Err(Refusal::NeverARoot(top.clone())));
        let beside = folders.top.join("beside");
        std::fs::create_dir(&beside)?;
        let refused = served(None, &[beside], &folders.home, || Ok(home.clone()));
        assert_eq!(refused, Err(Refusal::NeverARoot(top)));
        Ok(())
    }

    /// A turned-off repository is refused as turned off; a folder in no set-up repository, named
    /// by `--root`, as not set up, the current folder not read; and with no root served and a
    /// current folder that cannot be read, the first root's refusal is said, or, with no root,
    /// why there is no folder.
    #[test]
    fn each_refusal_says_why() -> TestResult {
        let folders = folders()?;
        let off = set_up(&folders.top.join("off"), &folders.home, true)?;
        let never = folders.top.join("never");
        std::fs::create_dir(&never)?;
        assert_eq!(
            served(None, std::slice::from_ref(&off), &folders.home, unreadable),
            Err(Refusal::TurnedOff(off.clone()))
        );
        let refused = served(Some(&never), &[], &folders.home, || {
            Err(io::Error::other(
                "the current folder is not read with --root",
            ))
        });
        assert_eq!(refused, Err(Refusal::NotSetUp(never)));
        assert_eq!(
            served(None, &[], &folders.home, unreadable),
            Err(Refusal::NoFolder("deleted".to_owned()))
        );
        assert_eq!(
            Refusal::TurnedOff(PathBuf::from("/srv/repo")).to_string(),
            format!(
                "Cortex is turned off in {}; tell the user that `cortex on` there turns it on",
                Path::new("/srv/repo").display()
            )
        );
        Ok(())
    }

    /// A root in a linked working tree of a set-up repository serves the repository, with the
    /// linked tree's top as the folder paths are read from.
    #[test]
    fn a_linked_working_tree_serves_its_repository() -> TestResult {
        let folders = folders()?;
        let main = folders.top.join("main");
        std::fs::create_dir(&main)?;
        let git = |args: &[&str], folder: &Path| -> TestResult {
            let status = std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .current_dir(folder)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("HOME", &folders.home)
                .env("USERPROFILE", &folders.home)
                .status()?;
            if !status.success() {
                return Err(format!("git {args:?} failed").into());
            }
            Ok(())
        };
        git(&["init", "-q"], &main)?;
        git(&["commit", "-q", "--allow-empty", "-m", "first"], &main)?;
        let linked = folders.top.join("linked");
        let spelled = linked.to_str().ok_or("a path that is not UTF-8")?;
        git(&["worktree", "add", "-q", spelled], &main)?;
        let main = set_up(&main, &folders.home, false)?;
        let linked = canonical_root(&linked)?;
        let home = folders.home.clone();
        let served = served(None, std::slice::from_ref(&linked), &folders.home, || {
            Ok(home)
        })?;
        assert_eq!(served, (main, linked));
        Ok(())
    }
}
