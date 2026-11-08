//! What `cortex init` writes for each agent: the registry of agents, how each is found, and the
//! files that set Cortex up in it. Adding an agent means one file here and one registry line.

mod augment;
pub(crate) mod claude;
mod codex;
pub(crate) mod command;
mod devin;
mod droid;
mod gemini;
mod junie;
mod qwen;
pub(crate) mod settings;

use std::path::Path;

use cortex_core::{Agent, find_program};

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
    /// What the user does themselves for the hooks to run, said after init: Cortex never writes an
    /// agent's trust or approval.
    pub(crate) told: &'static [&'static str],
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
];

/// The setup of the agent named `name` on the command line.
pub(crate) fn setup(name: &str) -> Option<&'static Setup> {
    SETUPS
        .iter()
        .copied()
        .find(|setup| setup.agent.name() == name)
}

/// Whether `setup`'s agent is on this machine: one of its folders in `home` holding anything but
/// the hooks file Cortex writes there, or one of its programs on the `PATH` outside `repository`.
pub(crate) fn is_found(setup: &Setup, home: &Path, repository: &Path) -> bool {
    let written = setup.hooks.as_ref().map(|hooks| hooks.user);
    let has_folder = setup
        .found_by
        .folders
        .iter()
        .any(|folder| holds_agents_own(home, folder, written));
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

/// Whether the folder `folder` of the home folder `home` holds an entry besides `written`, the
/// home-relative file Cortex writes hooks to: an empty folder, or one holding only Cortex's own
/// file, is no sign of the agent, since an earlier `cortex init --agents` makes it.
fn holds_agents_own(home: &Path, folder: &str, written: Option<&str>) -> bool {
    let Ok(entries) = std::fs::read_dir(home.join(folder)) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = format!("{folder}/{}", entry.file_name().to_string_lossy());
        Some(path.as_str()) != written
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

    /// Every agent is named once, by the name its adapter has, which `cortex hook --agent` takes;
    /// every hooks file's style names the same agent.
    #[test]
    fn every_setup_is_its_adapter_s() {
        let mut names: Vec<&str> = SETUPS.iter().map(|setup| setup.agent.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SETUPS.len());
        for setup in SETUPS {
            let name = setup.agent.name();
            assert!(cortex_hook::adapter(name).is_some(), "{name}");
            if let Some(hooks) = &setup.hooks {
                assert_eq!(hooks.style.agent, name);
            }
        }
    }

    /// Every agent's file of the repository's that init writes hooks to is one git is told to
    /// leave out, since a hook names this machine's program.
    #[test]
    fn every_local_file_is_left_out_of_git() {
        for setup in SETUPS {
            if let Some(project) = setup.hooks.as_ref().and_then(|hooks| hooks.project) {
                assert!(
                    cortex_core::IGNORE_LOCAL_FILES.contains(&project),
                    "{project}"
                );
            }
        }
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
            }),
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
}
