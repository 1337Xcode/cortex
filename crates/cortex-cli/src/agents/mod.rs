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

use cortex_core::find_program;

use self::settings::Style;

/// One agent `cortex init` sets up.
#[derive(Debug)]
pub(crate) struct Setup {
    /// The agent's name on Cortex's command line, as its adapter names it.
    pub(crate) name: &'static str,
    /// The agent's name as people write it.
    pub(crate) title: &'static str,
    /// How the agent is found on this machine.
    pub(crate) found_by: FoundBy,
    /// Where its hooks go and how, if Cortex writes them into a settings file.
    pub(crate) hooks: Option<HooksFile>,
    /// What the user does themselves for the hooks to run, said after init: Cortex never writes an
    /// agent's trust or approval.
    pub(crate) told: &'static [&'static str],
}

/// How an agent is found: any of its folders in the user's home, or any of its programs on the
/// `PATH`.
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
    SETUPS.iter().copied().find(|setup| setup.name == name)
}

/// Whether `setup`'s agent is on this machine: one of its folders in `home`, or one of its
/// programs on the `PATH` outside `repository`.
pub(crate) fn is_found(setup: &Setup, home: &Path, repository: &Path) -> bool {
    let has_folder = setup
        .found_by
        .folders
        .iter()
        .any(|folder| home.join(folder).is_dir());
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
        let mut names: Vec<&str> = SETUPS.iter().map(|setup| setup.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SETUPS.len());
        for setup in SETUPS {
            assert!(cortex_hook::adapter(setup.name).is_some(), "{}", setup.name);
            if let Some(hooks) = &setup.hooks {
                assert_eq!(hooks.style.agent, setup.name);
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

    /// An agent is found by its folder in the home folder; one with neither folder nor program is
    /// not.
    #[test]
    fn an_agent_is_found_by_its_folder() -> std::io::Result<()> {
        let home = tempfile::tempdir()?;
        let repository = tempfile::tempdir()?;
        let unknown = Setup {
            name: "none",
            title: "None",
            found_by: FoundBy {
                folders: &[".none-agent"],
                programs: &["cortex-test-no-such-agent"],
            },
            hooks: None,
            told: &[],
        };
        assert!(!is_found(&unknown, home.path(), repository.path()));
        std::fs::create_dir(home.path().join(".none-agent"))?;
        assert!(is_found(&unknown, home.path(), repository.path()));
        Ok(())
    }
}
