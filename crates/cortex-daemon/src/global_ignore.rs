//! The user's global gitignore, found as git finds it: the file `core.excludesFile` names in the
//! user's git configuration, else in the system's, else git's default, `$XDG_CONFIG_HOME/git/ignore`
//! or `~/.config/git/ignore`.
//!
//! Git itself is asked, since its configuration can be spread over files that include one another;
//! asked this way it reads its configuration and runs nothing else. It is asked for those two
//! scopes only, never the repository's own: a repository's `.git/config` could otherwise name the
//! file the daemon opens, a network path on Windows or a pipe that never ends. A repository keeps
//! its own patterns in `.git/info/exclude`, which the walk reads. Git runs in the repository, so
//! that the user's includes conditioned on it apply, and a relative path is resolved from the
//! repository's root, as git resolves it there.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, bail};

use crate::child;

/// How long git may take to answer before the global gitignore is left unread.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// The exit code of `git config --get` for a key that is not set.
const UNSET: i32 = 1;

/// The most of git's answer read: a path, with room to spare.
const MAX_ANSWER_BYTES: u64 = 64 * 1024;

/// What the user's git configuration says of `core.excludesFile`.
#[derive(Debug, PartialEq, Eq)]
enum Setting {
    /// It names this file.
    File(PathBuf),
    /// It is set to nothing: git then reads no global gitignore.
    Empty,
    /// It is not set: git then reads its default file.
    Unset,
}

/// The user's global gitignore for the repository at `root`, as git would read it there: the file
/// `core.excludesFile` names in the user's configuration, else in the system's (relative to `root`
/// when relative), none when it is set to nothing, else git's default file. A file that does not
/// exist is fine: the walk skips it, as git does.
///
/// # Errors
///
/// Git does not answer within [`GIT_TIMEOUT`], or fails other than by finding the setting unset:
/// then which file git reads is not known.
pub(crate) fn find(root: &Path) -> anyhow::Result<Option<PathBuf>> {
    let mut setting = excludes_file(root, "--global")?;
    if setting == Setting::Unset {
        setting = excludes_file(root, "--system")?;
    }
    Ok(match setting {
        Setting::File(path) => Some(root.join(path)),
        Setting::Empty => None,
        Setting::Unset => default_file(std::env::var_os("XDG_CONFIG_HOME"), home_folder()),
    })
}

/// What git, run in `root` and reading only the configuration of `scope` (`--global` or
/// `--system`), says `core.excludesFile` is. Without git installed, no configuration can name a
/// file: the setting counts as unset, and the default is where git would look.
fn excludes_file(root: &Path, scope: &str) -> anyhow::Result<Setting> {
    let mut git = Command::new("git");
    // INVARIANT: local-only (git is asked to read its configuration and nothing else: no
    // command that reaches a remote)
    git.args(["config", scope, "--get", "--type=path", "core.excludesFile"])
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    let answer = match child::capture(&mut git, GIT_TIMEOUT, MAX_ANSWER_BYTES) {
        Ok(answer) => answer,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Setting::Unset),
        Err(error) => return Err(error).context("running git to find the global gitignore"),
    };
    if answer.status.code() == Some(UNSET) {
        return Ok(Setting::Unset);
    }
    if !answer.status.success() {
        bail!("git could not read its configuration ({})", answer.status);
    }
    if answer.is_cut {
        bail!("git's answer is longer than a path");
    }
    let answer = String::from_utf8(answer.stdout).context("git's answer is not UTF-8")?;
    let path = answer.trim_end_matches(['\r', '\n']);
    Ok(if path.is_empty() {
        Setting::Empty
    } else {
        Setting::File(PathBuf::from(path))
    })
}

/// The user's home folder as git finds it: `HOME`; on Windows, without it, `HOMEDRIVE` followed by
/// `HOMEPATH`, else `USERPROFILE`.
fn home_folder() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(home) = set("HOME") {
        return Some(PathBuf::from(home));
    }
    if !cfg!(windows) {
        return None;
    }
    match (set("HOMEDRIVE"), set("HOMEPATH")) {
        (Some(drive), Some(path)) => {
            let mut home = drive;
            home.push(path);
            Some(PathBuf::from(home))
        }
        _ => set("USERPROFILE").map(PathBuf::from),
    }
}

/// Git's global gitignore when `core.excludesFile` is unset: `git/ignore` in `xdg_config_home`
/// when that is set and not empty, else `.config/git/ignore` in `home`.
fn default_file(xdg_config_home: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    match xdg_config_home.filter(|folder| !folder.is_empty()) {
        Some(folder) => Some(PathBuf::from(folder).join("git").join("ignore")),
        None => home.map(|home| home.join(".config").join("git").join("ignore")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Git's default follows `XDG_CONFIG_HOME` when it is set and not empty, else the home folder;
    /// with neither there is none.
    #[test]
    fn the_default_file_is_git_s() {
        let home = Some(PathBuf::from("/home/dev"));
        assert_eq!(
            default_file(Some(OsString::from("/xdg")), home.clone()),
            Some(PathBuf::from("/xdg").join("git").join("ignore"))
        );
        assert_eq!(
            default_file(Some(OsString::new()), home.clone()),
            Some(PathBuf::from("/home/dev/.config/git/ignore"))
        );
        assert_eq!(
            default_file(None, home),
            Some(PathBuf::from("/home/dev/.config/git/ignore"))
        );
        assert_eq!(default_file(None, None), None);
    }
}
