//! What uninstall without `--scope user` says of the user's own files that still run Cortex for an
//! agent in every repository, which only `--scope user` takes out (init writes them there for an
//! agent with no file of the repository's, and an agent's MCP server always): each agent's user
//! hooks file that holds Cortex's hooks, and each folder of Cortex's own files and each MCP
//! settings file a run of init recorded writing, not undone since, that still holds them. The
//! files are those the manifests name, so that no system folder is looked up to say this.

use std::collections::BTreeSet;
use std::path::Path;

use cortex_core::{MAX_SETTINGS_BYTES, UserFolder, read_text_file};

use super::runs;
use crate::agents::{self, Setup, mcp, settings};
use crate::backup::Kind;

/// The notes for the user's files that hold Cortex's hooks, files or MCP server, one per hooks
/// file, folder of files or settings file; none when none does.
pub(super) fn users_hooks(user: &UserFolder, program: &str) -> Vec<String> {
    let Some(home) = std::env::home_dir() else {
        return Vec::new();
    };
    let mut notes: Vec<String> = agents::SETUPS
        .iter()
        .filter_map(|setup| hooks_note(setup, &home, program))
        .collect();
    notes.extend(written_notes(user, program));
    notes
}

/// The note for `setup`'s agent's user hooks file under `home`, when it holds Cortex's hooks or
/// cannot be read.
fn hooks_note(setup: &Setup, home: &Path, program: &str) -> Option<String> {
    let hooks = setup.hooks.as_ref()?;
    let path = hooks
        .user
        .split('/')
        .fold(home.to_path_buf(), |path, part| path.join(part));
    let title = setup.agent.title();
    let found = read_text_file(&path, MAX_SETTINGS_BYTES)
        .map_err(anyhow::Error::from)
        .and_then(|text| {
            text.map_or(Ok(Vec::new()), |text| {
                settings::cortex_commands(&text, program, &hooks.style)
            })
        });
    match found {
        Ok(commands) if commands.is_empty() => None,
        Ok(_) => Some(format!(
            "note: {} holds Cortex's {title} hooks, which run in every repository; `cortex \
             uninstall --scope user` takes them out",
            path.display()
        )),
        Err(error) => Some(format!(
            "note: could not tell whether {} holds Cortex's {title} hooks: {error:#}",
            path.display()
        )),
    }
}

/// The notes for the files of Cortex's own and the MCP servers runs of init on the user's files
/// recorded writing, not undone since, that are still there: one per agent's folder of files and
/// per settings file.
fn written_notes(user: &UserFolder, program: &str) -> Vec<String> {
    let mut passed_over = Vec::new();
    let runs = match runs::of_user(user, &mut passed_over) {
        Ok(runs) => runs,
        Err(error) => {
            return vec![format!(
                "note: could not tell which of your own files hold Cortex's files or MCP servers: \
                 {error:#}"
            )];
        }
    };
    let mut said = BTreeSet::new();
    let mut notes = Vec::new();
    for changed in runs
        .iter()
        .filter(|run| !run.manifest.undone)
        .flat_map(|run| &run.manifest.files)
    {
        let path = Path::new(&changed.path);
        let (setup, what, shown) = match &changed.kind {
            Kind::AgentFile { agent } => {
                let Some(setup) = agents::setup(agent).filter(|_| path.is_file()) else {
                    continue;
                };
                let what = setup.own.as_ref().map_or("files", |own| own.what);
                (setup, what, path.parent().unwrap_or(path))
            }
            Kind::AgentMcp { agent } => {
                let Some(setup) = agents::setup(agent) else {
                    continue;
                };
                let holds = setup.mcp.as_ref().is_some_and(|file| {
                    read_text_file(path, MAX_SETTINGS_BYTES)
                        .ok()
                        .flatten()
                        .is_some_and(|text| mcp::holds_cortex(&text, program, file))
                });
                if !holds {
                    continue;
                }
                (setup, "MCP server", path)
            }
            _ => continue,
        };
        if said.insert(shown.to_path_buf()) {
            notes.push(format!(
                "note: {} holds Cortex's {} {what}, which serves every repository; `cortex \
                 uninstall --scope user` takes it out",
                shown.display(),
                setup.agent.title()
            ));
        }
    }
    notes
}
