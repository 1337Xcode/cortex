//! Where `cortex init` sets Cortex up: the top of the git repository that holds the folder it
//! starts from, as `git rev-parse --show-toplevel` says. The folder itself, with why, when git is
//! not on the `PATH`, names no repository or does not answer, names a top that does not hold the
//! folder (a repository's `core.worktree` can name any folder), or names the user's home folder or
//! a folder holding it. Never the home folder itself or a folder holding it: the home folder's
//! `.cortex` is the user's own, and no repository Cortex serves is there.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, bail};
use cortex_core::canonical_root;

/// How long git has to say where the repository's top is.
const GIT_WITHIN: Duration = Duration::from_secs(5);

/// The most of git's answer read: one path, however long.
const MAX_ANSWER_BYTES: u64 = 65_536;

/// The name git's program has on this platform.
const GIT: &[&str] = if cfg!(windows) {
    &["git.exe"]
} else {
    &["git"]
};

/// Where Cortex is set up, and why it is not git's repository top when it is not.
#[derive(Debug)]
pub(super) struct Root {
    /// The folder, in its canonical form.
    pub(super) folder: PathBuf,
    /// Why it is the folder init started from rather than the top of a git repository.
    pub(super) without_git: Option<String>,
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
    let root = match git_top(&start, &home) {
        Ok(top) if !start.starts_with(&top) => alone(
            start,
            "git names a repository top that does not hold this folder",
        ),
        Ok(top) if home.starts_with(&top) => alone(
            start,
            "git's repository here is the home folder or holds it, which Cortex never sets up",
        ),
        Ok(top) => Root {
            folder: top,
            without_git: None,
        },
        Err(why) => alone(start, &why),
    };
    if home.starts_with(&root.folder) {
        bail!(
            "Cortex never sets up the home folder or a folder holding it ({}); run `cortex init` \
             in a repository",
            root.folder.display()
        );
    }
    Ok(root)
}

/// The folder `start` set up alone, for the reason `why`.
fn alone(start: PathBuf, why: &str) -> Root {
    Root {
        folder: start,
        without_git: Some(why.to_owned()),
    }
}

/// The top of the git repository that holds `start`, in its canonical form, or why there is none
/// to use. Git is the program the `PATH` names outside every repository around `start` (see
/// [`repositories_around`]), run in `start` with no repository named by the environment, so that
/// it finds the one that holds `start`.
fn git_top(start: &Path, home: &Path) -> Result<PathBuf, String> {
    let git = crate::program::on_path(GIT, repositories_around(start, home))
        .ok_or_else(|| "git is not on the PATH".to_owned())?;
    let mut command = Command::new(git);
    command
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    let captured = cortex_daemon::capture(&mut command, GIT_WITHIN, MAX_ANSWER_BYTES)
        .map_err(|error| format!("git did not say where the repository is ({error})"))?;
    if !captured.status.success() {
        return Err(format!(
            "git named no repository here (it ended with {})",
            captured.status
        ));
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
