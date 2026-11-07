//! Cortex's hooks in the settings file an agent reads them from: for a project's scope, the file of
//! the repository's the agent does not share through git (Claude Code's `.claude/settings.local.json`),
//! and the user's file (`~/.claude/settings.json`, `~/.codex/hooks.json`) for the user's scope or
//! for an agent with no such file of the repository's; merged with Cortex's current hooks (see
//! [`crate::agents::settings`]). A file the user made read-only is left as it is.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cortex_core::{MAX_SETTINGS_BYTES, read_text_file};

use super::{Change, Owner, Scope, place};
use crate::agents::{Setup, settings};
use crate::backup::Kind;

/// The settings file an agent's hooks go to, what it holds, and what it holds with Cortex's
/// current hooks.
pub(super) struct Settings {
    /// The agent.
    setup: &'static Setup,
    /// The file, links followed.
    pub(super) file: PathBuf,
    /// What it holds, or `None` when there is none.
    before: Option<String>,
    /// What it holds with Cortex's current hooks.
    after: String,
    /// Whose it is.
    owner: Owner,
}

/// The settings `scope` names for `setup`'s agent in the repository whose canonical root is `root`,
/// with the hooks of its adapter merged in; `None` for an agent whose hooks Cortex does not write
/// into a settings file. Nothing is written.
///
/// # Errors
///
/// No home folder for the user's file, a file that leads outside the repository, is a link to
/// nothing, is read-only, holds more than 1 MiB, or is not UTF-8 or the JSON the agent reads; or
/// this program's path cannot be written for the agent's shell.
pub(super) fn planned(
    scope: Scope,
    root: &Path,
    setup: &'static Setup,
) -> anyhow::Result<Option<Settings>> {
    let Some(hooks) = &setup.hooks else {
        return Ok(None);
    };
    let adapter = cortex_hook::adapter(setup.name)
        .with_context(|| format!("finding the {} adapter", setup.title))?;
    let (file, owner) = match (scope, hooks.project) {
        (Scope::Project, Some(project)) => {
            let (folder, name) = split(project);
            (
                place::file_in(root, folder, name)?,
                Owner::Repository { folder },
            )
        }
        _ => {
            let home = std::env::home_dir().context("finding the home folder")?;
            let (folder, name) = split(hooks.user);
            let folder = folder.map_or_else(|| home.clone(), |folder| home.join(folder));
            (place::file_beside(&folder, name)?, Owner::User)
        }
    };
    place::writable(&file)?;
    let before = read_text_file(&file, MAX_SETTINGS_BYTES)?;
    let program = crate::program::this_program()?;
    let after = settings::merged(before.as_deref(), &program, adapter.hooks(), &hooks.style)
        .with_context(|| format!("reading the {} settings {}", setup.title, file.display()))?;
    Ok(Some(Settings {
        setup,
        file,
        before,
        after,
        owner,
    }))
}

/// `path`'s folder, if it has one, and its file name.
fn split(path: &'static str) -> (Option<&'static str>, &'static str) {
    match path.rsplit_once('/') {
        Some((folder, name)) => (Some(folder), name),
        None => (None, path),
    }
}

impl Settings {
    /// Whether the file holds Cortex's current hooks already.
    pub(super) fn is_current(&self) -> bool {
        self.before.as_deref() == Some(self.after.as_str())
    }

    /// The repository's local file init makes, which git should then leave out; none when the file
    /// is there already or is the user's.
    pub(super) fn made_in_repository(&self) -> Option<&'static str> {
        let project = self.setup.hooks.as_ref().and_then(|hooks| hooks.project)?;
        (self.before.is_none() && matches!(self.owner, Owner::Repository { .. })).then_some(project)
    }

    /// What init says when the file holds Cortex's current hooks already.
    pub(super) fn current(&self) -> String {
        format!(
            "{}: the hooks in {} are current",
            self.setup.title,
            self.file.display()
        )
    }

    /// The change that writes Cortex's current hooks.
    pub(super) fn into_change(self) -> Change {
        let kind = match self.setup.name {
            "claude" => Kind::ClaudeSettings,
            agent => Kind::AgentSettings {
                agent: agent.to_owned(),
            },
        };
        Change {
            said: format!(
                "{}: hooks written to {}",
                self.setup.title,
                self.file.display()
            ),
            file: self.file,
            kind,
            before: self.before,
            after: self.after,
            owner: self.owner,
        }
    }
}
