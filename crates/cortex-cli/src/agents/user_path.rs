//! Where a user's file of an agent's is: under the home folder, under the user's configuration
//! folder (`$XDG_CONFIG_HOME` when it names an absolute folder, which Amp, OpenCode and Kilo Code
//! follow, else `~/.config`), or under the user's documents folder as the system names it, which
//! Cline reads its hooks from: on Windows the folder PowerShell's `GetFolderPath('MyDocuments')`
//! gives, which follows a folder the user moved (to OneDrive, say), asked not to check that it
//! exists, since it answers nothing for a folder not made yet; on Linux the
//! `XDG_DOCUMENTS_DIR` of `user-dirs.dirs` in the configuration folder (`~/Dokumente`, say), or
//! `~/Documents` without one; on macOS `~/Documents`. The documents folder is found only when
//! init, doctor or uninstall asks, never on a hook's path.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Context as _;

/// What a user's path is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Under {
    /// The home folder.
    Home,
    /// The user's configuration folder.
    Config,
    /// The user's documents folder.
    Documents,
}

/// A path of the user's, with `/` between folders, relative to what it is under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UserPath {
    /// What it is relative to.
    pub(crate) under: Under,
    /// The path.
    pub(crate) path: &'static str,
}

impl UserPath {
    /// A path under the home folder.
    pub(crate) const fn home(path: &'static str) -> Self {
        Self {
            under: Under::Home,
            path,
        }
    }

    /// A path under the user's configuration folder.
    pub(crate) const fn config(path: &'static str) -> Self {
        Self {
            under: Under::Config,
            path,
        }
    }

    /// A path under the user's documents folder.
    pub(crate) const fn documents(path: &'static str) -> Self {
        Self {
            under: Under::Documents,
            path,
        }
    }

    /// The path relative to the home folder where it is when nothing moved its base: for an agent
    /// found by its folders, which never asks the system.
    pub(crate) fn in_home(self) -> String {
        match self.under {
            Under::Home => self.path.to_owned(),
            Under::Config => format!(".config/{}", self.path),
            Under::Documents => format!("Documents/{}", self.path),
        }
    }

    /// The path for this user.
    ///
    /// # Errors
    ///
    /// No home folder, or the documents folder cannot be found.
    pub(crate) fn resolved(self) -> anyhow::Result<PathBuf> {
        let home = std::env::home_dir().context("finding the home folder")?;
        let base = match self.under {
            Under::Home => home,
            Under::Config => config_folder(&home, std::env::var_os("XDG_CONFIG_HOME")),
            Under::Documents => documents_folder(&home)?,
        };
        Ok(joined(&base, self.path))
    }
}

/// `relative`, with `/` between folders, inside `folder`.
fn joined(folder: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(folder.to_path_buf(), |path, part| path.join(part))
}

/// The user's configuration folder for the home folder `home` and the value `xdg` of
/// `XDG_CONFIG_HOME`: that value when it names an absolute folder, which is all the variable may
/// name, else `.config` in the home folder.
pub(crate) fn config_folder(home: &Path, xdg: Option<OsString>) -> PathBuf {
    xdg.map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
}

/// How long PowerShell has to name the documents folder.
#[cfg(windows)]
const DOCUMENTS_WITHIN: std::time::Duration = std::time::Duration::from_secs(5);

/// The most bytes of PowerShell's answer read: a path.
#[cfg(windows)]
const MAX_ANSWER_BYTES: u64 = 4_096;

/// The documents folder Windows names for this user, as PowerShell's
/// `[Environment]::GetFolderPath('MyDocuments', 'DoNotVerify')` gives it: the system's own
/// PowerShell, run as an argument list with no profile and no input, given [`DOCUMENTS_WITHIN`].
///
/// # Errors
///
/// `SystemRoot` is not set, PowerShell cannot run or does not end in time, or names no absolute
/// folder.
#[cfg(windows)]
fn documents_folder(_: &Path) -> anyhow::Result<PathBuf> {
    let root = std::env::var_os("SystemRoot").context("`SystemRoot` is not set")?;
    let powershell = joined(
        Path::new(&root),
        "System32/WindowsPowerShell/v1.0/powershell.exe",
    );
    let mut command = std::process::Command::new(&powershell);
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "[Environment]::GetFolderPath('MyDocuments', 'DoNotVerify')",
    ]);
    let captured = cortex_core::capture(&mut command, DOCUMENTS_WITHIN, MAX_ANSWER_BYTES)
        .with_context(|| format!("asking {} for the documents folder", powershell.display()))?;
    let answer = String::from_utf8(captured.stdout).unwrap_or_default();
    let folder = PathBuf::from(answer.trim());
    anyhow::ensure!(
        captured.status.success() && !captured.is_cut && folder.is_absolute(),
        "PowerShell named no documents folder"
    );
    Ok(folder)
}

/// The documents folder macOS gives every user.
///
/// # Errors
///
/// None.
#[cfg(target_os = "macos")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the other systems' versions of the same function can fail"
)]
fn documents_folder(home: &Path) -> anyhow::Result<PathBuf> {
    Ok(home.join("Documents"))
}

/// The documents folder `user-dirs.dirs` in the configuration folder of the home folder `home`
/// names, or `~/Documents` when there is no such file or it names none.
///
/// # Errors
///
/// The file is there but cannot be read as small text.
#[cfg(all(unix, not(target_os = "macos")))]
fn documents_folder(home: &Path) -> anyhow::Result<PathBuf> {
    let file = config_folder(home, std::env::var_os("XDG_CONFIG_HOME")).join("user-dirs.dirs");
    let text = cortex_core::read_text_file(&file, cortex_core::MAX_SETTINGS_BYTES)?;
    Ok(documents_in(text.as_deref(), home))
}

/// The documents folder the `user-dirs.dirs` text `text` names, its `$HOME` the home folder
/// `home`: its last `XDG_DOCUMENTS_DIR="..."` line naming an absolute folder, or `~/Documents`.
#[cfg(all(unix, not(target_os = "macos")))]
fn documents_in(text: Option<&str>, home: &Path) -> PathBuf {
    let mut named = text.into_iter().flat_map(str::lines).filter_map(|line| {
        let value = line.trim().strip_prefix("XDG_DOCUMENTS_DIR=")?;
        let value = value.strip_prefix('"')?.strip_suffix('"')?;
        let folder = match value.strip_prefix("$HOME") {
            Some(rest) => PathBuf::from(format!("{}{rest}", home.display())),
            None => PathBuf::from(value),
        };
        folder.is_absolute().then_some(folder)
    });
    named.next_back().unwrap_or_else(|| home.join("Documents"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `XDG_CONFIG_HOME` counts only when it names an absolute folder.
    #[test]
    fn the_configuration_folder_follows_an_absolute_xdg_folder() {
        let home = Path::new("/home/ann");
        assert_eq!(config_folder(home, None), Path::new("/home/ann/.config"));
        assert_eq!(
            config_folder(home, Some("relative/config".into())),
            Path::new("/home/ann/.config")
        );
        let absolute = std::env::temp_dir().join("xdg");
        assert_eq!(config_folder(home, Some(absolute.clone().into())), absolute);
    }

    /// The paths an agent is found by, before anything moved their base.
    #[test]
    fn a_path_in_the_home_folder_is_its_default() {
        assert_eq!(
            UserPath::config("amp/plugins").in_home(),
            ".config/amp/plugins"
        );
        assert_eq!(
            UserPath::documents("Cline/Hooks").in_home(),
            "Documents/Cline/Hooks"
        );
        assert_eq!(UserPath::home(".openclaw").in_home(), ".openclaw");
    }

    /// The documents folder `user-dirs.dirs` names, typed here by hand: `$HOME` expanded, an
    /// absolute folder as it is, the last line counting; a relative one, none, or no file gives
    /// `~/Documents`.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn user_dirs_name_the_documents_folder() {
        let home = Path::new("/home/ann");
        let text = "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n\
                    XDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n";
        assert_eq!(
            documents_in(Some(text), home),
            Path::new("/home/ann/Dokumente")
        );
        let text = "XDG_DOCUMENTS_DIR=\"$HOME/a\"\nXDG_DOCUMENTS_DIR=\"/data/docs\"\n";
        assert_eq!(documents_in(Some(text), home), Path::new("/data/docs"));
        for text in [Some("XDG_DOCUMENTS_DIR=\"docs\"\n"), Some("# none\n"), None] {
            assert_eq!(documents_in(text, home), Path::new("/home/ann/Documents"));
        }
    }
}
