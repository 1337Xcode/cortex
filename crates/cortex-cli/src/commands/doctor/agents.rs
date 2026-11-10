//! The check on the hooks of every agent but Claude Code that a run of `cortex init` set up for
//! this repository, as init's manifests in the user's backup folder say (read as uninstall reads
//! them, a manifest this build cannot read passed over, and a run uninstall has undone left out):
//! each file a run wrote an agent's hooks to still holds the hook of every event the agent's
//! adapter registers, running this binary, as this build writes them, and the user has not turned
//! them off there. The fix is init for that agent again, or, for hooks the user turned off, the
//! user's own switch, which init leaves as it is. Claude Code's hooks have checks of their own,
//! and an agent this build no longer sets up is not checked.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context as _;
use cortex_core::{MAX_SETTINGS_BYTES, UserFolder, read_text_file};

use super::{Context, Finding};
use crate::agents::{self, HooksFile, Setup, settings};
use crate::backup::Kind;
use crate::commands::uninstall::runs::{self, Run};

/// Each agent's hooks are current in every file a run of init wrote them to for this repository.
pub(super) fn current(context: &Context) -> Vec<Finding> {
    let written = match written_files(&context.root) {
        Ok(written) => written,
        Err(error) => {
            return vec![Finding::Fail {
                what: format!("cannot read init's manifests: {error:#}"),
                fix: "make the user's `.cortex/backups` folder readable".to_owned(),
            }];
        }
    };
    written
        .iter()
        .filter_map(|(agent, path)| {
            let setup = agents::setup(agent)?;
            let hooks = setup.hooks.as_ref()?;
            Some(checked(context, setup, hooks, Path::new(path)))
        })
        .collect()
}

/// The agents (but Claude Code) and the files the runs of init that name the repository at `root`
/// wrote their hooks to, each once, in name order.
///
/// # Errors
///
/// No home folder, or the backup folder or a manifest cannot be read.
fn written_files(root: &Path) -> anyhow::Result<BTreeSet<(String, String)>> {
    let user = UserFolder::of_current_user().context("finding the home folder")?;
    let mut passed_over = Vec::new();
    let mut runs: Vec<Run> = runs::of_repository(&user, root, &mut passed_over)?;
    runs.extend(
        runs::of_user(&user, &mut passed_over)?
            .into_iter()
            .filter(|run| Path::new(&run.manifest.repository) == root),
    );
    Ok(runs
        .iter()
        .filter(|run| !run.manifest.undone)
        .flat_map(|run| &run.manifest.files)
        .filter_map(|changed| match &changed.kind {
            Kind::AgentSettings { agent } => Some((agent.clone(), changed.path.clone())),
            Kind::ClaudeSettings | Kind::Gitignore { .. } | Kind::CortexConfig | Kind::Folder => {
                None
            }
        })
        .collect())
}

/// What `path`, where a run of init wrote `setup`'s hooks in the way `hooks` says, holds of them.
fn checked(context: &Context, setup: &Setup, hooks: &HooksFile, path: &Path) -> Finding {
    let title = setup.agent.title();
    let shown = path.display();
    let fix = init_command(context, setup, hooks, path);
    let Some(adapter) = cortex_hook::adapter(setup.agent.name()) else {
        return Finding::Fail {
            what: format!("{title}: this build has no adapter for it"),
            fix,
        };
    };
    let text = match read_text_file(path, MAX_SETTINGS_BYTES) {
        Ok(Some(text)) => text,
        Ok(None) => {
            return Finding::Fail {
                what: format!("{title}: {shown}, where init wrote its hooks, is gone"),
                fix,
            };
        }
        Err(error) => {
            return Finding::Fail {
                what: format!("{title}: cannot read {shown}: {error}"),
                fix: format!("make {shown} readable, then {fix}"),
            };
        }
    };
    if settings::is_turned_off(&text, &context.program, &hooks.style).is_ok_and(|is_off| is_off) {
        return Finding::Fail {
            what: format!(
                "{title}: Cortex's hooks in {shown} are turned off there (`\"enabled\": false`)"
            ),
            fix: format!(
                "take `\"enabled\": false` off Cortex's hooks in {shown} for {title} to run them; \
                 init leaves that switch as it is"
            ),
        };
    }
    match settings::is_current(&text, &context.program, adapter.hooks(), &hooks.style) {
        Ok(true) => Finding::Pass(format!("{title}: Cortex's hooks are current in {shown}")),
        Ok(false) => Finding::Fail {
            what: format!(
                "{title}: Cortex's hooks in {shown} are not what this build writes: one is \
                 missing or changed, or they run another binary"
            ),
            fix,
        },
        Err(error) => Finding::Fail {
            what: format!("{title}: {shown} is not the JSON {title} reads: {error:#}"),
            fix: format!("repair {shown}, then {fix}"),
        },
    }
}

/// The command that writes `setup`'s hooks to `path` again for the repository doctor checks: with
/// `--scope user` when the agent has a file of the repository's and `path` is not in it.
fn init_command(context: &Context, setup: &Setup, hooks: &HooksFile, path: &Path) -> String {
    let scope = if hooks.project.is_some() && !path.starts_with(&context.root) {
        " --scope user"
    } else {
        ""
    };
    format!(
        "run `cortex init --agents {}{scope} --root {}`",
        setup.agent.name(),
        context.root.display()
    )
}
