//! Cortex's hooks in the settings file an agent reads them from: for a project's scope, the file of
//! the repository's the agent does not share through git (Claude Code's `.claude/settings.local.json`),
//! and the user's file (`~/.claude/settings.json`, `~/.codex/hooks.json`) for the user's scope or
//! for an agent with no such file of the repository's; merged with Cortex's current hooks (see
//! [`crate::agents::settings`]). A file the user made read-only is left as it is.

use std::path::Path;

use anyhow::{Context as _, bail};
use cortex_core::{MAX_SETTINGS_BYTES, read_text_file};

use super::agent_file::{AgentFile, What};
use super::{Owner, Scope, place};
use crate::agents::{Setup, settings};
use crate::backup::Kind;

/// The settings `scope` names for `setup`'s agent in the repository whose canonical root is `root`,
/// or the user's own settings when there is no repository (`cortex init --global`), with the hooks
/// of its adapter merged in; `None` for an agent whose hooks Cortex does not write into a settings
/// file. Nothing is written.
///
/// # Errors
///
/// No home folder for the user's file, a file that leads outside the repository, is a link to
/// nothing, is read-only, holds more than 1 MiB, or is not UTF-8 or the JSON the agent reads; or
/// this program's path cannot be written for the agent's shell.
pub(super) fn planned(
    scope: Scope,
    root: Option<&Path>,
    setup: &'static Setup,
) -> anyhow::Result<Option<AgentFile>> {
    let Some(hooks) = &setup.hooks else {
        return Ok(None);
    };
    let adapter = cortex_hook::adapter(setup.agent.name())
        .with_context(|| format!("finding the {} adapter", setup.agent.title()))?;
    let (file, owner) = match (scope, hooks.project, root) {
        (Scope::Project, Some(project), Some(root)) => {
            let (folder, name) = split(project);
            (
                place::file_in(root, folder, name)?,
                Owner::Repository { folder },
            )
        }
        _ => {
            let home = std::env::home_dir().context("finding the home folder")?;
            let (folder, name) = split(hooks.user);
            let folder = folder.map_or_else(|| home.clone(), |folder| place::joined(&home, folder));
            (place::file_beside(&folder, name)?, Owner::User)
        }
    };
    place::writable(&file)?;
    let before = read_text_file(&file, MAX_SETTINGS_BYTES)?;
    if before.is_none()
        && owner == Owner::User
        && let Some(read_instead) = hooks.read_instead
    {
        refuse_hidden_hooks(read_instead, setup)?;
    }
    let program = crate::program::this_program()?;
    if let (Some(project), Some(root)) = (hooks.project, root) {
        refuse_both_scopes((root, project), owner, setup, &program)?;
    }
    let after = settings::merged(before.as_deref(), &program, adapter.hooks(), &hooks.style)
        .with_context(|| {
            format!(
                "reading the {} settings {}",
                setup.agent.title(),
                file.display()
            )
        })?;
    let kind = match setup.agent.name() {
        "claude" => Kind::ClaudeSettings,
        agent => Kind::AgentSettings {
            agent: agent.to_owned(),
        },
    };
    let ignore_line = hooks
        .project
        .filter(|_| matches!(owner, Owner::Repository { .. }));
    Ok(Some(AgentFile {
        setup,
        file,
        before,
        after,
        owner,
        kind,
        what: What::Hooks,
        ignore_line,
        is_executable: false,
    }))
}

/// Refuses when the file of the other scope, `project` in the repository at `root` when `owner` is
/// the user and the user's file otherwise, holds Cortex's hooks for `setup`'s agent, run by
/// `program` or an older Cortex: the agent runs the hooks of both files, so each event would run
/// Cortex twice. A file of the other scope that is not the JSON the agent reads runs nothing.
///
/// # Errors
///
/// No home folder, a file that cannot be read, or one holding Cortex's hooks.
fn refuse_both_scopes(
    (root, project): (&Path, &str),
    owner: Owner,
    setup: &Setup,
    program: &str,
) -> anyhow::Result<()> {
    let Some(hooks) = &setup.hooks else {
        return Ok(());
    };
    let (other, flag) = match owner {
        Owner::User => (place::joined(root, project), ""),
        Owner::Repository { .. } => {
            let home = std::env::home_dir().context("finding the home folder")?;
            (place::joined(&home, hooks.user), " --scope user")
        }
    };
    if holds_cortex_hooks(&other, setup, program)? {
        let title = setup.agent.title();
        bail!(
            "{} holds Cortex's {title} hooks already, and {title} runs the hooks of both files, \
             so each event would run Cortex twice; take those out first (`cortex uninstall{flag}` \
             undoes what init wrote there), or keep them and set {title} up in that scope",
            other.display()
        );
    }
    Ok(())
}

/// Whether the settings file `path` of `setup`'s agent holds Cortex's hooks, run by `program` or an
/// older Cortex. A file that is not there, or not the JSON the agent reads, runs none.
///
/// # Errors
///
/// The file cannot be read.
pub(super) fn holds_cortex_hooks(
    path: &Path,
    setup: &Setup,
    program: &str,
) -> anyhow::Result<bool> {
    let Some(hooks) = &setup.hooks else {
        return Ok(false);
    };
    let Some(text) = read_text_file(path, MAX_SETTINGS_BYTES)? else {
        return Ok(false);
    };
    Ok(settings::cortex_commands(&text, program, &hooks.style)
        .is_ok_and(|commands| !commands.is_empty()))
}

/// Refuses when `read_instead`, a file of the home folder that `setup`'s agent reads hooks from
/// while its own hooks file is absent, holds hooks: making that file would hide them. A file that
/// is not JSON cannot say, and is refused too.
///
/// # Errors
///
/// No home folder, a file that cannot be read, or one holding hooks or not JSON.
fn refuse_hidden_hooks(read_instead: &str, setup: &Setup) -> anyhow::Result<()> {
    let home = std::env::home_dir().context("finding the home folder")?;
    let path = read_instead
        .split('/')
        .fold(home, |path, part| path.join(part));
    let Some(text) = read_text_file(&path, MAX_SETTINGS_BYTES)? else {
        return Ok(());
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let holds = serde_json::from_str::<serde_json::Value>(text).map(|value| {
        value
            .get("hooks")
            .is_some_and(|hooks| !hooks.as_object().is_some_and(serde_json::Map::is_empty))
    });
    let title = setup.agent.title();
    match holds {
        Ok(false) => Ok(()),
        Ok(true) => bail!(
            "{} holds hooks, which {title} reads only while it has no hooks file of its own: \
             Cortex's would hide them; move them into that file first",
            path.display()
        ),
        Err(error) => bail!(
            "{} is not JSON ({error}), so whether it holds hooks {title} would stop reading \
             cannot be told",
            path.display()
        ),
    }
}

/// `path`'s folder, if it has one, and its file name.
fn split(path: &'static str) -> (Option<&'static str>, &'static str) {
    match path.rsplit_once('/') {
        Some((folder, name)) => (Some(folder), name),
        None => (None, path),
    }
}
