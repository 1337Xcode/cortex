//! Cortex's hooks in the Claude Code settings file a scope names: the repository's
//! `.claude/settings.local.json` for a project's, the user's `~/.claude/settings.json` for the
//! user's, merged with Cortex's current hooks (see [`crate::claude_settings`]). A file the user
//! made read-only is left as it is.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cortex_core::{MAX_SETTINGS_BYTES, read_text_file};
use cortex_hook::Adapter;

use super::{Change, Owner, Scope, place};
use crate::claude_settings;

/// The settings file the hooks go to, what it holds, and what it holds with Cortex's current
/// hooks.
pub(super) struct Settings {
    /// The file, links followed.
    pub(super) file: PathBuf,
    /// What it holds, or `None` when there is none.
    before: Option<String>,
    /// What it holds with Cortex's current hooks.
    after: String,
    /// Whose it is.
    owner: Owner,
}

/// The settings `scope` names for the repository whose canonical root is `root`, with the hooks
/// of `adapter` merged in. Nothing is written.
///
/// # Errors
///
/// No home folder for the user's scope, a file that leads outside the repository, is a link to
/// nothing, is read-only, holds more than 1 MiB, or is not UTF-8 or the JSON Claude Code reads.
pub(super) fn planned(
    scope: Scope,
    root: &Path,
    adapter: &dyn Adapter,
) -> anyhow::Result<Settings> {
    let (file, owner) = match scope {
        Scope::Project => (
            place::file_in(root, Some(".claude"), "settings.local.json")?,
            Owner::Repository {
                folder: Some(".claude"),
            },
        ),
        Scope::User => {
            let home = std::env::home_dir().context("finding the home folder")?;
            (
                place::file_beside(&home.join(".claude"), "settings.json")?,
                Owner::User,
            )
        }
    };
    place::writable(&file)?;
    let before = read_text_file(&file, MAX_SETTINGS_BYTES)?;
    let program = crate::program::this_program()?;
    let after = claude_settings::merged(before.as_deref(), &program, adapter.hooks())
        .with_context(|| format!("reading the Claude Code settings {}", file.display()))?;
    Ok(Settings {
        file,
        before,
        after,
        owner,
    })
}

impl Settings {
    /// Whether the file holds Cortex's current hooks already.
    pub(super) fn is_current(&self) -> bool {
        self.before.as_deref() == Some(self.after.as_str())
    }

    /// Whether init makes the repository's local settings, which git should then leave out.
    pub(super) fn is_made_in_repository(&self) -> bool {
        self.before.is_none() && matches!(self.owner, Owner::Repository { .. })
    }

    /// The change that writes Cortex's current hooks.
    pub(super) fn into_change(self) -> Change {
        Change {
            said: format!("Claude Code: hooks written to {}", self.file.display()),
            file: self.file,
            before: self.before,
            after: self.after,
            owner: self.owner,
        }
    }
}
