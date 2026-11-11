//! Where `cortex init` sets Cortex up: the top of the git repository that holds the folder it
//! starts from, as `git rev-parse --show-toplevel` says. The folder itself, with why, when git is
//! not on the `PATH`, names no repository or does not answer, names a top that does not hold the
//! folder (a repository's `core.worktree` can name any folder), or names the user's home folder or
//! a folder holding it. Never the home folder itself or a folder holding it: the home folder's
//! `.cortex` is the user's own, and no repository Cortex serves is there.
//!
//! In a linked working tree (`git worktree add`) of a repository with a main working tree, the
//! repository Cortex sets up is the main tree, which every working tree of it shares, and the
//! linked working tree is where the agents' own files go: no `.cortex` is made in it, which would
//! make it a repository of its own.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use cortex_core::{canonical_root, git_layout, is_never_a_root};

use super::git;

/// Where Cortex is set up, why it is not git's repository top when it is not, and where no git
/// that runs for it may be.
#[derive(Debug)]
pub(super) struct Root {
    /// The folder, in its canonical form.
    pub(super) folder: PathBuf,
    /// Why it is the folder init started from rather than the top of a git repository.
    pub(super) without_git: Option<String>,
    /// The folder no git that runs for this repository may be inside (see
    /// [`repositories_around`]).
    pub(super) git_bound: PathBuf,
    /// The linked working tree init started in, canonical, when it started in one: `folder` is
    /// then its repository's main working tree.
    pub(super) linked: Option<PathBuf>,
    /// Whether Cortex sets the repository up on its own, for an agent that works in it (see
    /// [`super::adopt`]): its ignore lines then go to the exclude file git keeps, never to a file
    /// its team commits.
    pub(super) is_adopted: bool,
}

impl Root {
    /// The working tree init started in: where the agents' files of the repository go.
    pub(super) fn worktree(&self) -> &Path {
        self.linked.as_deref().unwrap_or(&self.folder)
    }
}

/// Where `cortex init`, started from the folder `start`, sets Cortex up for the user whose home
/// folder is `home`.
///
/// # Errors
///
/// `start` or `home` cannot be found, or the folder would be the home folder or hold it.
pub(super) fn find(start: &Path, home: &Path) -> anyhow::Result<Root> {
    let start =
        canonical_root(start).with_context(|| format!("finding the folder {}", start.display()))?;
    let home = canonical_root(home)
        .with_context(|| format!("finding the home folder {}", home.display()))?;
    let git_bound = repositories_around(&start, &home).to_path_buf();
    let mut linked = None;
    let (folder, without_git) = match git_top(&start, &git_bound) {
        Ok(top) if !start.starts_with(&top) => (
            start,
            Some("git names a repository top that does not hold this folder"),
        ),
        Ok(top) if home.starts_with(&top) => (
            start,
            Some(
                "git's repository here is the home folder or holds it, which Cortex never sets up",
            ),
        ),
        Ok(top) => match main_tree_of(&top) {
            Some(main) => {
                linked = Some(top);
                (main, None)
            }
            None => (top, None),
        },
        Err(why) => {
            return checked(
                Root {
                    folder: start,
                    without_git: Some(why),
                    git_bound,
                    linked: None,
                    is_adopted: false,
                },
                &home,
            );
        }
    };
    checked(
        Root {
            folder,
            without_git: without_git.map(str::to_owned),
            git_bound,
            linked,
            is_adopted: false,
        },
        &home,
    )
}

/// The main working tree of the repository whose linked working tree's top is `top`; none for a
/// main working tree, or a linked one whose repository has no main working tree (a bare one).
fn main_tree_of(top: &Path) -> Option<PathBuf> {
    git_layout(top)
        .filter(cortex_core::GitLayout::is_linked)?
        .main_root
}

/// `root`, unless its folder is the home folder `home`, a folder holding it, or a file system's
/// root.
///
/// # Errors
///
/// The folder is one of those.
fn checked(root: Root, home: &Path) -> anyhow::Result<Root> {
    if is_never_a_root(&root.folder, home) {
        bail!(
            "Cortex never sets up the home folder, a folder holding it, or a file system's root \
             ({}); run `cortex init` in a repository",
            root.folder.display()
        );
    }
    Ok(root)
}

/// The top of the git repository that holds `start`, in its canonical form, or why there is none
/// to use. Git runs as [`git::run`] runs it, outside `bound`.
fn git_top(start: &Path, bound: &Path) -> Result<PathBuf, String> {
    let captured = git::run(&["rev-parse", "--show-toplevel"], start, bound)?;
    if !captured.status.success() {
        return Err("not a git repository".to_owned());
    }
    if captured.is_cut {
        return Err("git's answer is longer than any path".to_owned());
    }
    let top = String::from_utf8(captured.stdout)
        .map_err(|_| "git named a repository top that is not UTF-8 text".to_owned())?;
    let top = top.trim_end_matches(['\n', '\r']);
    canonical_root(Path::new(top))
        .map_err(|error| format!("git named a repository top that cannot be found ({error})"))
}

/// The folder no git that runs for `start` may be inside: the outermost folder at or above
/// `start` and below `home` that holds a `.git`, since git's answer may be any of them; `start`
/// itself when none does.
fn repositories_around<'a>(start: &'a Path, home: &Path) -> &'a Path {
    start
        .ancestors()
        .take_while(|folder| !home.starts_with(folder))
        .filter(|folder| folder.join(".git").exists())
        .last()
        .unwrap_or(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Below the home folder, a repository inside another one: git may run from neither, so the
    /// outer one bounds it; a folder in no repository bounds it alone; the home folder's own
    /// `.git` and anything above it count for nothing.
    #[test]
    fn the_outermost_repository_below_home_bounds_git() -> Result<(), Box<dyn std::error::Error>> {
        let base = tempfile::tempdir()?;
        let home = base.path().join("home");
        let outer = home.join("outer");
        let inner = outer.join("inner");
        let start = inner.join("src");
        std::fs::create_dir_all(&start)?;
        for folder in [
            base.path(),
            home.as_path(),
            outer.as_path(),
            inner.as_path(),
        ] {
            std::fs::create_dir_all(folder.join(".git"))?;
        }
        assert_eq!(repositories_around(&start, &home), outer.as_path());
        let loose = home.join("loose");
        std::fs::create_dir(&loose)?;
        assert_eq!(repositories_around(&loose, &home), loose.as_path());
        Ok(())
    }
}
