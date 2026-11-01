//! The Claude Code settings files `cortex doctor` reads, highest precedence first: the managed
//! ones an organization installs, then the repository's local and shared settings, then the
//! user's. Each is read as `cortex init` reads a settings file: at most 1 MiB of UTF-8 text.

use std::fs;
use std::path::{Path, PathBuf};

use crate::claude_settings;

/// Which settings a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    /// An organization's, installed on the machine.
    Managed,
    /// The repository's own, not committed: `.claude/settings.local.json`.
    Local,
    /// The repository's shared settings: `.claude/settings.json`.
    Project,
    /// The user's: `~/.claude/settings.json`.
    User,
}

/// One settings file and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SettingsFile {
    /// Which settings it holds.
    pub(super) scope: Scope,
    /// Where it is.
    pub(super) path: PathBuf,
    /// Its text; `None` when there is no file; the reason when it cannot be read.
    pub(super) text: Result<Option<String>, String>,
}

/// The settings files of the repository at `root` and of the user, highest precedence first.
pub(super) fn files(root: &Path) -> Vec<SettingsFile> {
    let mut files: Vec<SettingsFile> = managed_files()
        .into_iter()
        .map(|path| read(Scope::Managed, path))
        .collect();
    let claude = root.join(".claude");
    files.push(read(Scope::Local, claude.join("settings.local.json")));
    files.push(read(Scope::Project, claude.join("settings.json")));
    if let Some(home) = std::env::home_dir() {
        files.push(read(
            Scope::User,
            home.join(".claude").join("settings.json"),
        ));
    }
    files
}

/// The managed settings files: `managed-settings.json` in the platform's managed folder, then the
/// files of its `managed-settings.d` folder, in name order.
fn managed_files() -> Vec<PathBuf> {
    let folder = managed_folder();
    let mut files = vec![folder.join("managed-settings.json")];
    if let Ok(entries) = fs::read_dir(folder.join("managed-settings.d")) {
        let mut dropped: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        dropped.sort();
        files.extend(dropped);
    }
    files
}

/// The folder Claude Code reads managed settings from on this platform.
fn managed_folder() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\Program Files\ClaudeCode")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode")
    } else {
        PathBuf::from("/etc/claude-code")
    }
}

/// The settings file of `scope` at `path`, read.
fn read(scope: Scope, path: PathBuf) -> SettingsFile {
    let text = claude_settings::read(&path).map_err(|error| format!("{error:#}"));
    SettingsFile { scope, path, text }
}
