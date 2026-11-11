//! What `cortex init` writes for each agent: the registry of agents, how each is found, and the
//! files that set Cortex up in it: hooks merged into a settings file, files of Cortex's own (a
//! plugin, hook scripts, a rules file), Cortex's MCP server merged into JSON settings, and what
//! the user adds to a file Cortex does not write. Adding an agent means one file here and one
//! registry line.

mod aider;
mod amp;
mod antigravity;
mod augment;
pub(crate) mod claude;
mod cline;
mod codex;
pub(crate) mod command;
mod continue_;
mod copilot;
mod crush;
mod cursor;
mod devin;
mod droid;
mod entries;
mod gemini;
mod goose;
pub(crate) mod handler;
mod hermes;
mod junie;
mod kilo;
mod kiro;
pub(crate) mod mcp;
mod openclaw;
mod opencode;
mod openhands;
pub(crate) mod own;
mod qwen;
mod roo;
pub(crate) mod rules;
pub(crate) mod settings;
mod trae;
pub(crate) mod user_path;
mod warp;
mod windsurf;
mod yaml;
mod zed;

use std::path::Path;

use cortex_core::{Agent, find_program};

use self::mcp::McpFile;
use self::own::{OwnFiles, OwnKind, UsersFolder};
use self::settings::Style;

/// One agent `cortex init` sets up.
#[derive(Debug)]
pub(crate) struct Setup {
    /// The agent: its name on Cortex's command line, as its adapter names it, and as people write
    /// it.
    pub(crate) agent: Agent,
    /// How the agent is found on this machine.
    pub(crate) found_by: FoundBy,
    /// Where its hooks go and how, if Cortex writes them into a settings file.
    pub(crate) hooks: Option<HooksFile>,
    /// The files of Cortex's own it reads, if Cortex writes any: a plugin, hook scripts.
    pub(crate) own: Option<OwnFiles>,
    /// Where Cortex's MCP server goes, if Cortex writes it into JSON settings.
    pub(crate) mcp: Option<McpFile>,
    /// The lines the user adds to a file of the agent's that Cortex does not write, if any.
    pub(crate) told_lines: Option<ToldLines>,
    /// What the user does themselves for the hooks to run, said after init: Cortex never writes an
    /// agent's trust or approval.
    pub(crate) told: &'static [&'static str],
}

/// Lines for the user to add to a file of an agent's that Cortex does not write (YAML, which
/// Cortex has no writer for): init says them, and doctor looks for them.
#[derive(Debug)]
pub(crate) struct ToldLines {
    /// The file, relative to the home folder with `/` between folders.
    pub(crate) file: &'static str,
    /// Why Cortex writes nothing there itself, said with the lines.
    pub(crate) why: &'static str,
    /// The lines for this program, each ending with a line break; an error when the program's
    /// path cannot be written for the agent.
    pub(crate) lines: fn(&str) -> anyhow::Result<String>,
    /// The texts the lines put in the file for this program, each of which doctor looks for.
    pub(crate) signs: fn(&str) -> anyhow::Result<Vec<String>>,
}

/// How an agent is found: any of its folders in the user's home holding the agent's own files, or
/// any of its programs on the `PATH`.
#[derive(Debug)]
pub(crate) struct FoundBy {
    /// Folders relative to the home folder, with `/` between folders.
    pub(crate) folders: &'static [&'static str],
    /// Program names without an extension; on Windows `.exe` and `.cmd` are looked for.
    pub(crate) programs: &'static [&'static str],
}

/// The settings file an agent reads its hooks from, for a repository and for the user, and how it
/// holds them.
#[derive(Debug)]
pub(crate) struct HooksFile {
    /// The file in the repository, relative to its root, with `/` between folders; none when the
    /// agent has no file of the repository's that is not shared through git, since a hook names
    /// this machine's program: the user's file is written then, whatever the scope.
    pub(crate) project: Option<&'static str>,
    /// The user's file, relative to the home folder, with `/` between folders.
    pub(crate) user: &'static str,
    /// How the file holds Cortex's hooks.
    pub(crate) style: Style,
    /// The user's file, relative to the home folder, that the agent reads hooks from while
    /// [`HooksFile::user`] is absent: when it holds hooks, making the user's file would hide
    /// them, so init does not.
    pub(crate) read_instead: Option<&'static str>,
}

/// Every agent `cortex init` sets up, in the order it says them.
pub(crate) const SETUPS: &[&Setup] = &[
    &claude::SETUP,
    &codex::SETUP,
    &qwen::SETUP,
    &gemini::SETUP,
    &droid::SETUP,
    &devin::SETUP,
    &augment::SETUP,
    &junie::SETUP,
    &cursor::SETUP,
    &copilot::SETUP,
    &antigravity::SETUP,
    &kiro::SETUP,
    &trae::SETUP,
    &crush::SETUP,
    &openhands::SETUP,
    &opencode::SETUP,
    &kilo::SETUP,
    &amp::SETUP,
    &openclaw::SETUP,
    &cline::SETUP,
    &hermes::SETUP,
    &goose::SETUP,
    &zed::SETUP,
    &continue_::SETUP,
    &warp::SETUP,
    &windsurf::SETUP,
    &roo::SETUP,
    &aider::SETUP,
];

/// The setup of the agent named `name` on the command line.
pub(crate) fn setup(name: &str) -> Option<&'static Setup> {
    SETUPS
        .iter()
        .copied()
        .find(|setup| setup.agent.name() == name)
}

/// Whether `setup`'s agent is on this machine: one of its folders in `home` holding anything but
/// the files Cortex writes there, or one of its programs on the `PATH` outside `repository`.
pub(crate) fn is_found(setup: &Setup, home: &Path, repository: &Path) -> bool {
    let written = users_files(setup);
    let has_folder = setup
        .found_by
        .folders
        .iter()
        .any(|folder| holds_agents_own(home, folder, &written));
    has_folder || {
        let names: Vec<String> = setup
            .found_by
            .programs
            .iter()
            .flat_map(|program| program_names(program))
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        find_program(&names, repository).is_some()
    }
}

/// The files of the home folder, relative to it with `/` between folders, that init may write for
/// `setup`'s agent at the user's scope: its hooks file, the files of Cortex's own in its folder,
/// and its MCP settings.
pub(crate) fn users_files(setup: &Setup) -> Vec<String> {
    let mut files: Vec<String> = setup
        .hooks
        .iter()
        .map(|hooks| hooks.user.to_owned())
        .collect();
    if let Some(own) = &setup.own
        && let Some(folder) = own.user.path()
    {
        // The files' names do not depend on the program they run.
        let names = (own.files)("cortex").unwrap_or_default();
        let folder = folder.in_home();
        files.extend(names.iter().map(|file| format!("{folder}/{}", file.name)));
    }
    if let Some(mcp) = &setup.mcp {
        files.extend(mcp.files().iter().map(|file| file.in_home()));
    }
    files
}

/// What the user is to do for `setup`'s agent, this binary being `program`, that Cortex never does
/// and doctor cannot check, each as init says it: the rules to add where Cortex writes none for the
/// user, when `is_for_user`; the MCP entry to add by hand; and the agent's own steps.
pub(crate) fn steps(setup: &Setup, program: &str, is_for_user: bool) -> Vec<String> {
    let mut steps = Vec::new();
    if is_for_user && let Some(UsersFolder::Told(told)) = setup.own.as_ref().map(|own| own.user) {
        steps.push(told.to_owned());
    }
    steps.extend(
        setup
            .mcp
            .as_ref()
            .and_then(|file| mcp::by_hand_step(file, program)),
    );
    steps.extend(setup.told.iter().map(|step| (*step).to_owned()));
    steps
}

/// Whether Cortex writes anything for `setup`'s agent at the user's scope: hooks, a file of its own
/// in the user's folder, or its MCP server in the user's settings.
pub(crate) fn writes_for_user(setup: &Setup) -> bool {
    setup.hooks.is_some()
        || setup
            .own
            .as_ref()
            .is_some_and(|own| matches!(own.user, UsersFolder::At(_)))
        || setup
            .mcp
            .as_ref()
            .is_some_and(|file| !file.files().is_empty())
}

/// Each rules file of Cortex's own init may write in a repository for an agent of the registry,
/// relative to the repository's root with `/` between folders: git is told to leave each out
/// through the exclude file.
pub(crate) fn rules_files() -> Vec<String> {
    let mut paths = Vec::new();
    for own in SETUPS.iter().filter_map(|setup| setup.own.as_ref()) {
        if let (OwnKind::Rules, Some(project)) = (own.kind, own.project) {
            // A rules file's name does not depend on the program.
            let names = (own.files)("cortex").unwrap_or_default();
            paths.extend(names.iter().map(|file| format!("{project}/{}", file.name)));
        }
    }
    paths
}

/// Whether the folder `folder` of the home folder `home` holds an entry besides `written`, the
/// home-relative files Cortex writes, and the folders on the way to them: an empty folder, or one
/// holding only Cortex's own files, is no sign of the agent, since an earlier `cortex init
/// --agents` makes them.
fn holds_agents_own(home: &Path, folder: &str, written: &[String]) -> bool {
    let Ok(entries) = std::fs::read_dir(home.join(folder)) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = format!("{folder}/{}", entry.file_name().to_string_lossy());
        let is_on_the_way = written.iter().any(|written| {
            written
                .strip_prefix(path.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        });
        if is_on_the_way && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            return holds_agents_own(home, &path, written);
        }
        !written.contains(&path)
    })
}

/// The file names `program` has on this system: `program.exe` and `program.cmd` on Windows, where
/// tools installed with npm are `.cmd` files, and `program` elsewhere.
fn program_names(program: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{program}.exe"), format!("{program}.cmd")]
    } else {
        vec![program.to_owned()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every agent is named once; every agent Cortex writes hooks, a plugin or hook scripts for
    /// has an adapter of the same name, which `cortex hook --agent` takes, and every hooks file's
    /// style names that agent. An agent Cortex reaches through MCP alone needs none.
    #[test]
    fn every_setup_is_its_adapter_s() {
        let mut names: Vec<&str> = SETUPS.iter().map(|setup| setup.agent.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SETUPS.len());
        for setup in SETUPS {
            let name = setup.agent.name();
            let runs_cortex = setup.hooks.is_some()
                || setup
                    .own
                    .as_ref()
                    .is_some_and(|own| own.kind != OwnKind::Rules);
            assert!(
                !runs_cortex || cortex_hook::adapter(name).is_some(),
                "{name}"
            );
            if let Some(hooks) = &setup.hooks {
                assert_eq!(hooks.style.agent, name);
            }
        }
    }

    /// Every agent's file of the repository's that init writes hooks to, and every file of
    /// Cortex's own it writes in a repository's folder, is one git is told to leave out, since it
    /// names this machine's program or tells an agent to call a tool only this machine gives it:
    /// a rules file through the exclude file git never commits, every other file as Cortex's
    /// folder is.
    #[test]
    fn every_local_file_is_left_out_of_git() -> anyhow::Result<()> {
        let rules = rules_files();
        assert_eq!(
            rules,
            [
                ".continue/rules/cortex.md",
                ".windsurf/rules/cortex.md",
                ".roo/rules/cortex.md"
            ]
        );
        for setup in SETUPS {
            if let Some(project) = setup.hooks.as_ref().and_then(|hooks| hooks.project) {
                assert!(
                    cortex_core::IGNORE_LOCAL_FILES.contains(&project),
                    "{project}"
                );
            }
            if let Some(own) = &setup.own
                && let Some(folder) = own.project
            {
                for file in (own.files)("/opt/cortex")? {
                    let path = format!("{folder}/{}", file.name);
                    let lists = (
                        cortex_core::IGNORE_LOCAL_FILES.contains(&path.as_str()),
                        rules.contains(&path),
                    );
                    let expected = if own.kind == OwnKind::Rules {
                        (false, true)
                    } else {
                        (true, false)
                    };
                    assert_eq!(lists, expected, "{path}");
                }
            }
        }
        Ok(())
    }

    /// An agent is found by its folder in the home folder once the folder holds a file besides
    /// the hooks file Cortex writes there; an empty folder, one holding only that file, or no
    /// folder and no program, is not.
    #[test]
    fn an_agent_is_found_by_its_folder() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let repository = tempfile::tempdir()?;
        let unknown = Setup {
            agent: Agent::Other,
            found_by: FoundBy {
                folders: &[".none-agent"],
                programs: &["cortex-test-no-such-agent"],
            },
            hooks: Some(HooksFile {
                project: None,
                user: ".none-agent/hooks.json",
                style: claude::STYLE,
                read_instead: None,
            }),
            own: None,
            mcp: None,
            told_lines: None,
            told: &[],
        };
        assert!(!is_found(&unknown, home.path(), repository.path()));
        let folder = home.path().join(".none-agent");
        std::fs::create_dir(&folder)?;
        assert!(!is_found(&unknown, home.path(), repository.path()));
        std::fs::write(folder.join("hooks.json"), "{}")?;
        assert!(!is_found(&unknown, home.path(), repository.path()));
        std::fs::write(folder.join("config.toml"), "")?;
        assert!(is_found(&unknown, home.path(), repository.path()));
        Ok(())
    }

    /// An agent whose hooks file Cortex writes in a folder of the agent's folder is not found by
    /// that file and the folder holding it, which init makes; anything else in either is a sign.
    #[test]
    fn an_agent_is_not_found_by_cortex_s_folder_of_hooks() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let repository = tempfile::tempdir()?;
        let unknown = Setup {
            agent: Agent::Other,
            found_by: FoundBy {
                folders: &[".none-agent"],
                programs: &[],
            },
            hooks: Some(HooksFile {
                project: None,
                user: ".none-agent/hooks/cortex.json",
                style: claude::STYLE,
                read_instead: None,
            }),
            own: None,
            mcp: None,
            told_lines: None,
            told: &[],
        };
        let hooks = home.path().join(".none-agent").join("hooks");
        std::fs::create_dir_all(&hooks)?;
        std::fs::write(hooks.join("cortex.json"), "{}")?;
        assert!(!is_found(&unknown, home.path(), repository.path()));
        std::fs::write(hooks.join("mine.json"), "{}")?;
        assert!(is_found(&unknown, home.path(), repository.path()));
        std::fs::remove_file(hooks.join("mine.json"))?;
        std::fs::write(home.path().join(".none-agent").join("hooks.txt"), "")?;
        assert!(is_found(&unknown, home.path(), repository.path()));
        Ok(())
    }

    /// Every agent's hooks file, written fresh, holds one hook for each event of its adapter's
    /// table and no other, with that event's matcher (on its group, on the handler, or on the named
    /// hook), each running `hook --agent <agent> --event <event>`. The file is read here by its
    /// documented shape, not by the code that writes it.
    #[test]
    fn every_agent_s_file_holds_its_adapter_s_events() -> anyhow::Result<()> {
        use serde_json::Value;
        let program = "/opt/cortex/bin/cortex";
        for setup in SETUPS {
            let name = setup.agent.name();
            let Some(hooks) = &setup.hooks else {
                continue;
            };
            let adapter = cortex_hook::adapter(name)
                .ok_or(name)
                .map_err(anyhow::Error::msg)?;
            let written = settings::merged(None, program, adapter.hooks(), &hooks.style)?;
            let written: Value = serde_json::from_str(&written)?;
            // (event, matcher, command line) for each hook found in the file.
            let mut found: Vec<(String, Option<String>, String)> = Vec::new();
            let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
            let events = match hooks.style.events {
                settings::Events::Triggers => {
                    for hook in written["hooks"].as_array().into_iter().flatten() {
                        found.push((
                            text(hook.get("trigger")).unwrap_or_default(),
                            text(hook.get("matcher")),
                            text(hook.pointer("/action/command")).unwrap_or_default(),
                        ));
                    }
                    None
                }
                settings::Events::Under(key) => written.get(key).cloned(),
                settings::Events::WholeFile | settings::Events::WholeFileOrHooks => {
                    Some(written.clone())
                }
            };
            let key = hooks.style.keys.command;
            for (event, items) in events.iter().filter_map(Value::as_object).flatten() {
                for item in items.as_array().into_iter().flatten() {
                    let handlers: Vec<&Value> = match item.get("hooks").and_then(Value::as_array) {
                        Some(handlers) => handlers.iter().collect(),
                        None => vec![item],
                    };
                    for handler in handlers {
                        let line = match &hooks.style.handler {
                            settings::Handler::Arguments => format!(
                                "{} {}",
                                text(handler.get(key)).unwrap_or_default(),
                                handler["args"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            ),
                            settings::Handler::CommandLine(_) => {
                                text(handler.get(key)).unwrap_or_default()
                            }
                        };
                        found.push((event.clone(), text(item.get("matcher")), line));
                    }
                }
            }
            let expected: Vec<(String, Option<String>, String)> = adapter
                .hooks()
                .iter()
                .map(|entry| {
                    (
                        // A file spelled as the user's is spelled in snake case when fresh.
                        match hooks.style.spelling {
                            settings::Spelling::AsAdapter => entry.event_name.to_owned(),
                            settings::Spelling::AsFile => entry
                                .event_name
                                .chars()
                                .enumerate()
                                .flat_map(|(at, char)| {
                                    let underscore =
                                        (at > 0 && char.is_ascii_uppercase()).then_some('_');
                                    underscore.into_iter().chain([char.to_ascii_lowercase()])
                                })
                                .collect(),
                        },
                        entry.matcher.map(str::to_owned),
                        format!("{program} hook --agent {name} --event {}", entry.event_name),
                    )
                })
                .collect();
            found.sort();
            let mut expected = expected;
            expected.sort();
            assert_eq!(found, expected, "{name}");
        }
        Ok(())
    }
}
