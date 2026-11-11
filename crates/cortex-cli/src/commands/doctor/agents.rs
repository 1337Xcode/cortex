//! The check on what runs of `cortex init` wrote for every agent but Claude Code for this
//! repository, as init's manifests in the user's backup folder say (read as uninstall reads them,
//! a manifest this build cannot read passed over, and a run uninstall has undone left out): each
//! file a run wrote an agent's hooks to still holds the hook of every event the agent's adapter
//! registers, running this binary, as this build writes them, and the user has not turned them off
//! there; each file of Cortex's own (a plugin, a hook script) holds what this build writes, and on
//! Unix a hook script is still executable, since that is the switch Cline turns a hook off with;
//! each MCP settings file holds Cortex's current server. The fix is init for that agent again, or,
//! for hooks the user turned off, the user's own switch, which init leaves as it is; what the user
//! is to do for the agent and doctor cannot check is said with it. Claude Code's hooks have checks
//! of their own, and an agent this build no longer sets up is not checked.
//!
//! An agent Cortex writes nothing for, whose lines init only says (see
//! [`crate::agents::ToldLines`]), is checked by those lines in its file, when the file holds any
//! of them or the agent is found on this machine.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context as _;
use cortex_core::{MAX_SETTINGS_BYTES, UserFolder, read_text_file};

use super::{Context, Finding};
use crate::agents::own::Signs;
use crate::agents::{self, Setup, ToldLines, mcp, own, settings};
use crate::backup::Kind;
use crate::commands::uninstall::runs::{self, Run};

/// What a run of init wrote to a file for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Written {
    /// The agent's hooks, in its settings.
    Hooks,
    /// A file of Cortex's own.
    File,
    /// Cortex's MCP server, in its settings.
    Mcp,
}

/// Each agent's hooks, files and MCP server are current in every file a run of init wrote them to
/// for this repository; then the agents init only tells the user about.
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
    let mut findings: Vec<Finding> = written
        .iter()
        .filter_map(|(agent, written, path)| {
            let setup = agents::setup(agent)?;
            let path = Path::new(path);
            match written {
                Written::Hooks => checked_hooks(context, setup, path),
                Written::File => checked_file(context, setup, path),
                Written::Mcp => checked_mcp(context, setup, path),
            }
        })
        .collect();
    let mut steps_said = BTreeSet::new();
    for (agent, written, _) in &written {
        let setup = agents::setup(agent).filter(|_| *written != Written::Hooks);
        if let Some(setup) = setup.filter(|_| steps_said.insert(agent)) {
            findings.extend(setup.told.iter().map(|step| {
                Finding::Pass(format!(
                    "{}: yours to do, which doctor cannot check: {step}",
                    setup.agent.title()
                ))
            }));
        }
    }
    findings.extend(
        agents::SETUPS
            .iter()
            .filter_map(|setup| told(context, setup)),
    );
    findings
}

/// The agents (but Claude Code), what was written for each, and the files the runs of init that
/// name the repository at `root` wrote it to, each once, in name order.
///
/// # Errors
///
/// No home folder, or the backup folder or a manifest cannot be read.
fn written_files(root: &Path) -> anyhow::Result<BTreeSet<(String, Written, String)>> {
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
        .filter_map(|changed| {
            let (agent, written) = match &changed.kind {
                Kind::AgentSettings { agent } => (agent, Written::Hooks),
                Kind::AgentFile { agent } => (agent, Written::File),
                Kind::AgentMcp { agent } => (agent, Written::Mcp),
                Kind::ClaudeSettings
                | Kind::Gitignore { .. }
                | Kind::CortexConfig
                | Kind::Folder => {
                    return None;
                }
            };
            Some((agent.clone(), written, changed.path.clone()))
        })
        .collect())
}

/// What `path` holds, read for `setup`'s agent; the failure to report when it is gone or cannot
/// be read.
fn read_for(setup: &Setup, path: &Path, what: &str, fix: &str) -> Result<String, Finding> {
    let title = setup.agent.title();
    let shown = path.display();
    match read_text_file(path, MAX_SETTINGS_BYTES) {
        Ok(Some(text)) => Ok(text),
        Ok(None) => Err(Finding::Fail {
            what: format!("{title}: {shown}, where init wrote its {what}, is gone"),
            fix: fix.to_owned(),
        }),
        Err(error) => Err(Finding::Fail {
            what: format!("{title}: cannot read {shown}: {error}"),
            fix: format!("make {shown} readable, then {fix}"),
        }),
    }
}

/// What `path`, where a run of init wrote `setup`'s hooks, holds of them; none for an agent whose
/// hooks Cortex no longer writes into a settings file.
fn checked_hooks(context: &Context, setup: &Setup, path: &Path) -> Option<Finding> {
    let hooks = setup.hooks.as_ref()?;
    let title = setup.agent.title();
    let shown = path.display();
    let fix = init_command(context, setup, Some(path));
    let Some(adapter) = cortex_hook::adapter(setup.agent.name()) else {
        return Some(Finding::Fail {
            what: format!("{title}: this build has no adapter for it"),
            fix,
        });
    };
    let text = match read_for(setup, path, "hooks", &fix) {
        Ok(text) => text,
        Err(finding) => return Some(finding),
    };
    if settings::is_turned_off(&text, &context.program, &hooks.style).is_ok_and(|is_off| is_off) {
        return Some(Finding::Fail {
            what: format!(
                "{title}: Cortex's hooks in {shown} are turned off there (`\"enabled\": false`)"
            ),
            fix: format!(
                "take `\"enabled\": false` off Cortex's hooks in {shown} for {title} to run them; \
                 init leaves that switch as it is"
            ),
        });
    }
    Some(
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
        },
    )
}

/// What `path`, a file of Cortex's own a run of init wrote for `setup`'s agent, holds; none for an
/// agent Cortex no longer writes such files for, or a file this build does not write.
fn checked_file(context: &Context, setup: &Setup, path: &Path) -> Option<Finding> {
    let own = setup.own.as_ref()?;
    let title = setup.agent.title();
    let shown = path.display();
    let fix = init_command(context, setup, Some(path));
    let written = match (own.files)(&context.program) {
        Ok(written) => written,
        Err(error) => {
            return Some(Finding::Fail {
                what: format!(
                    "{title}: this build cannot write its {}: {error:#}",
                    own.what
                ),
                fix,
            });
        }
    };
    let file = written
        .iter()
        .find(|file| path.ends_with(Path::new(&file.name)))?;
    // The agent's folder: the path without the file's own folders and name.
    let folder = path.ancestors().nth(file.name.split('/').count())?;
    if let Some(finding) = moved_away(context, setup, folder, path) {
        return Some(finding);
    }
    let text = match read_for(setup, path, own.what, &fix) {
        Ok(text) => text,
        Err(finding) => return Some(finding),
    };
    if text != file.text {
        let recorded = UserFolder::of_current_user()
            .and_then(|user| runs::own_files_written(&user).ok())
            .and_then(|recorded| recorded.get(path).cloned())
            .unwrap_or_default();
        let signs = Signs {
            recorded: &recorded,
            is_in_marked_folder: own::is_in_marked_folder(folder, file, &written),
        };
        return Some(Finding::Fail {
            what: format!(
                "{title}: {shown} is not the {} this build writes: it changed, or it runs another \
                 binary",
                own.what
            ),
            fix: if own::is_cortex_s(&text, file, signs) {
                fix
            } else {
                format!("move {shown} elsewhere if it is yours, then {fix}")
            },
        });
    }
    if file.is_executable && !is_executable(path) {
        return Some(Finding::Fail {
            what: format!(
                "{title}: Cortex's {} {shown} is turned off there (not executable)",
                own.what
            ),
            fix: format!(
                "turn it on in {title} (or make it executable) for {title} to run it; init leaves \
                 that switch as it is"
            ),
        });
    }
    Some(Finding::Pass(format!(
        "{title}: Cortex's {} is current in {shown}",
        own.what
    )))
}

/// The failure to report when `folder`, where a run of init wrote a file of Cortex's own at `path`
/// for `setup`'s agent, is not the user's folder the agent reads now (the documents folder moved,
/// or `XDG_CONFIG_HOME` changed); none for a folder of the repository's, or the one it reads.
fn moved_away(context: &Context, setup: &Setup, folder: &Path, path: &Path) -> Option<Finding> {
    let own = setup.own.as_ref()?;
    let user = own.user?;
    if own.project.is_some() && path.starts_with(&context.root) {
        return None;
    }
    let title = setup.agent.title();
    let fix = init_command(context, setup, Some(path));
    match user.resolved() {
        Ok(reads) if same_folder(&reads, folder) => None,
        Ok(reads) => Some(Finding::Fail {
            what: format!(
                "{title}: {} is not where {title} reads the user's {}s now, {}",
                path.display(),
                own.what,
                reads.display()
            ),
            fix,
        }),
        Err(error) => Some(Finding::Fail {
            what: format!(
                "{title}: the folder {title} reads the user's {}s from cannot be found: {error:#}",
                own.what
            ),
            fix: format!("set {title} up with `--scope project` in a repository instead"),
        }),
    }
}

/// Whether `reads`, a folder as the system names it, is `written`, the folder as init recorded it,
/// their links followed when they can be.
fn same_folder(reads: &Path, written: &Path) -> bool {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    reads == written || canonical(reads) == canonical(written)
}

/// Whether the file at `path` may be run, as its executable bit says.
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o100 != 0)
}

/// Whether the file at `path` may be run: always, where files carry no such bit.
#[cfg(not(unix))]
fn is_executable(_: &Path) -> bool {
    true
}

/// What `path`, the settings a run of init wrote `setup`'s MCP server to, holds of it; none for
/// an agent Cortex no longer writes one for.
fn checked_mcp(context: &Context, setup: &Setup, path: &Path) -> Option<Finding> {
    let file = setup.mcp.as_ref()?;
    let title = setup.agent.title();
    let shown = path.display();
    // Init writes the user's MCP settings whatever the scope.
    let fix = init_command(context, setup, None);
    let reads: Vec<std::path::PathBuf> = file
        .user
        .iter()
        .filter_map(|user| user.resolved().ok())
        .collect();
    if !reads.iter().any(|reads| same_folder(reads, path)) {
        return Some(Finding::Fail {
            what: format!(
                "{title}: {shown} is not a settings file {title} reads now; it reads {}",
                reads
                    .iter()
                    .map(|reads| reads.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" or ")
            ),
            fix,
        });
    }
    let text = match read_for(setup, path, "MCP server", &fix) {
        Ok(text) => text,
        Err(finding) => return Some(finding),
    };
    if mcp::is_turned_off(&text, &context.program, file)
        && let Some((key, off)) = file.off
    {
        return Some(Finding::Fail {
            what: format!(
                "{title}: Cortex's MCP server in {shown} is turned off there (`\"{key}\": {off}`)"
            ),
            fix: format!(
                "take `\"{key}\": {off}` off Cortex's MCP server in {shown} for {title} to start \
                 it; init leaves that switch as it is"
            ),
        });
    }
    Some(match mcp::is_current(&text, &context.program, file) {
        Ok(true) => Finding::Pass(format!(
            "{title}: Cortex's MCP server is current in {shown}"
        )),
        Ok(false) => Finding::Fail {
            what: format!(
                "{title}: Cortex's MCP server in {shown} is not what this build writes: it is \
                 missing or changed, or it runs another binary"
            ),
            fix,
        },
        Err(error) => Finding::Fail {
            what: format!("{title}: {shown} cannot take Cortex's MCP server: {error:#}"),
            fix: format!("repair {shown}, then {fix}"),
        },
    })
}

/// What the file of `setup`'s agent that init only tells the user about holds of Cortex's lines:
/// none when the agent has no such file, the file holds none of them and the agent is not found
/// on this machine.
fn told(context: &Context, setup: &Setup) -> Option<Finding> {
    let lines: &ToldLines = setup.told_lines.as_ref()?;
    let home = std::env::home_dir()?;
    let path = lines
        .file
        .split('/')
        .fold(home.clone(), |path, part| path.join(part));
    let title = setup.agent.title();
    let name = setup.agent.name();
    let shown = path.display();
    let signs = (lines.signs)(&context.program).ok()?;
    let text = read_text_file(&path, MAX_SETTINGS_BYTES)
        .ok()
        .flatten()
        .unwrap_or_default();
    let held = signs
        .iter()
        .filter(|sign| text.contains(sign.as_str()))
        .count();
    let steps: String = setup
        .told
        .iter()
        .map(|step| format!("; yours to do, which doctor cannot check: {step}"))
        .collect();
    if held == signs.len() {
        return Some(Finding::Pass(format!(
            "{title}: Cortex's lines are in {shown}{steps}"
        )));
    }
    if held > 0 {
        return Some(Finding::Fail {
            what: format!("{title}: {shown} holds some of Cortex's lines, not all this build says"),
            fix: format!("add the lines `cortex init --agents {name}` prints to {shown}"),
        });
    }
    agents::is_found(setup, &home, &context.root).then(|| {
        Finding::Pass(format!(
            "{title}: found; Cortex writes nothing for it, since {}: `cortex init --agents \
             {name}` prints the lines to add to {shown}",
            lines.why
        ))
    })
}

/// The command that writes `setup`'s files again for the repository doctor checks: with `--scope
/// user` when the agent has a file or folder of the repository's and `path`, the file to write, is
/// not in it (none: a file init writes at either scope).
fn init_command(context: &Context, setup: &Setup, path: Option<&Path>) -> String {
    let has_project = setup
        .hooks
        .as_ref()
        .is_some_and(|hooks| hooks.project.is_some())
        || setup.own.as_ref().is_some_and(|own| own.project.is_some());
    let is_users = path.is_some_and(|path| !path.starts_with(&context.root));
    let scope = if has_project && is_users {
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
