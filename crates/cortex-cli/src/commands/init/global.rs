//! `cortex init --global`: Cortex's hooks for every agent found on this machine (or those
//! `--agents` names) in the user's own settings, which serve every repository; the holdout question
//! asked once and kept in the user's `~/.cortex/config.toml`; then adoption turned on, the file
//! `~/.cortex/adopt` written last, once everything else is: from then on Cortex sets up on its own
//! each repository an agent works in that is not set up yet (see [`super::adopt`]). Each file is
//! backed up first, as init backs up the user's files, so that `cortex uninstall --global` gives
//! each back byte for byte and turns adoption off. Running it again changes nothing but forgets
//! the folders Cortex refused to set up on its own, so that a change of `adopt.exclude` counts for
//! them too.
//!
//! No repository runs Cortex from both scopes: an agent that reads hooks from a repository's own
//! file as well as the user's runs both, so each event there would run Cortex twice. Init for one
//! repository refuses when the user's file holds Cortex's hooks; `--global` refuses, before it
//! writes anything, when a repository init set up (as the user's backups name them) holds Cortex's
//! hooks in its own file for an agent it would set up, and names the uninstall that takes them out.

use std::collections::BTreeSet;
use std::io::{self, IsTerminal as _};

use anyhow::{Context as _, bail};
use cortex_core::{UserFolder, canonical_root, create_private_folder};

use super::consent::{self, ConfigFile};
use super::{
    Found, Owner, Scope, apply, check_claude_code, hooks, known_agents, place, planned_hooks,
    selected, settings_changes,
};
use crate::agents::Setup;
use crate::commands::uninstall::runs::{self, Run};
use crate::{adoption, output};

/// What `--global` says once adoption is on.
const TURNED_ON: &str = "adoption: on; Cortex sets up on its own each git repository below your \
     home folder that an agent works in, but those `adopt.exclude` names; `cortex off` turns it \
     off in one, `cortex uninstall --global` everywhere";

/// Sets Cortex up for the user, as the module says, and says what it did.
///
/// # Errors
///
/// No home folder, a name that is no agent Cortex sets up, a Claude Code too old, a question left
/// unanswered, a file that cannot be read or is read-only, or a backup or a write that fails.
pub(super) fn run(is_yes: bool, names: &[String]) -> anyhow::Result<()> {
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let home = std::env::home_dir().context("finding the home folder")?;
    let home = canonical_root(&home)
        .with_context(|| format!("finding the home folder {}", home.display()))?;
    create_private_folder(user.path())
        .with_context(|| format!("creating {}", user.path().display()))?;
    // The user's own folder bounds where an agent's program may be: the home folder would leave
    // out every program installed in it.
    let (setups, are_named) = selected(names, user.path())?;
    if setups.is_empty() {
        output::line(&format!(
            "note: no agent Cortex sets up was found on this machine, so no hooks are written; \
             once one is installed, run `cortex init --global` again, or name one with \
             `--agents` ({})",
            known_agents()
        ))?;
    }
    if setups.iter().any(|setup| setup.agent.name() == "claude") {
        let adapter = cortex_hook::adapter("claude").context("finding the Claude Code adapter")?;
        let versions = adapter
            .versions()
            .context("the Claude Code adapter names no versions")?;
        check_claude_code(user.path(), versions)?;
    }
    let planned = planned_hooks(Scope::User, None, &setups, are_named)?;
    refuse_both_scopes(&user, &planned.set_up)?;
    for line in &planned.not_set_up {
        output::line(line)?;
    }
    let users_config = Found::read(user.config_path())?;
    let none = ConfigFile {
        text: None,
        origin: "",
    };
    // Sharing is not asked: a team decides it for one repository, by `cortex init` there.
    let kept = consent::kept(users_config.config(), none, Some(false))?;
    let stdin = io::stdin();
    let mut terminal = stdin.is_terminal().then(|| stdin.lock());
    let answers = consent::answers(kept, is_yes, terminal.as_mut())?;
    let mut changes = settings_changes(planned.settings)?;
    if let Some(after) = consent::with_answers(users_config.config(), kept, answers)? {
        changes.push(users_config.changed(after, Owner::User, "answer kept in"));
    }
    apply((&home, None), changes)?;
    adoption::forget_all(&user)?;
    adoption::switch(&user.adopt_path(), true)?;
    output::line(TURNED_ON)?;
    for setup in &planned.set_up {
        for step in setup.told {
            output::line(&format!("{}: to do yourself: {step}", setup.agent.title()))?;
        }
    }
    Ok(())
}

/// Refuses when a repository that `cortex init` set up, as the backups in the user's folder
/// `user` name it, holds Cortex's hooks in its own file for an agent of `setups`, naming each such
/// file and the uninstall that takes the hooks out.
///
/// # Errors
///
/// This binary's path or a backup cannot be read, a settings file cannot be read, or one holds
/// Cortex's hooks.
fn refuse_both_scopes(user: &UserFolder, setups: &[&'static Setup]) -> anyhow::Result<()> {
    let program = crate::program::this_program()?;
    let mut passed_over = Vec::new();
    let runs = runs::of_repositories(user, &mut passed_over)?;
    let mut doubled = BTreeSet::new();
    for run in runs.iter().filter(|run| !run.manifest.undone) {
        for setup in setups {
            doubled.extend(doubled_in(run, setup, &program)?);
        }
    }
    if doubled.is_empty() {
        return Ok(());
    }
    bail!(
        "Cortex's hooks are in a repository's own settings already ({}); an agent runs the hooks \
         of both its repository's settings and the user's, so each event there would run Cortex \
         twice: take those out first, then run `cortex init --global` again",
        doubled.into_iter().collect::<Vec<_>>().join("; ")
    )
}

/// Each file of `run`'s repository, or of the linked working tree it started in, that holds
/// Cortex's hooks for `setup`'s agent, run by `program` or an older Cortex, said with the uninstall
/// that takes them out.
///
/// # Errors
///
/// A settings file cannot be read.
fn doubled_in(run: &Run, setup: &Setup, program: &str) -> anyhow::Result<Vec<String>> {
    let Some(project) = setup.hooks.as_ref().and_then(|hooks| hooks.project) else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for folder in &run.bounds {
        let file = place::joined(folder, project);
        if hooks::holds_cortex_hooks(&file, setup, program)? {
            found.push(format!(
                "{} holds {}'s; `cortex uninstall --root {}` takes them out",
                file.display(),
                setup.agent.title(),
                run.manifest.repository
            ));
        }
    }
    Ok(found)
}

/// Turns adoption off for the user whose folder is `user`, and says whether it was on: the
/// switch's file and the refusals go.
///
/// # Errors
///
/// A file or folder that is there cannot be removed.
pub(crate) fn switch_off(user: &UserFolder) -> anyhow::Result<bool> {
    let was_on = adoption::switch(&user.adopt_path(), false)?;
    adoption::forget_all(user)?;
    Ok(was_on)
}
