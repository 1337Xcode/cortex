//! Kiro (its editor and its `kiro-cli` command-line agent): its hooks in JSON files of a hooks
//! folder, `{"version": "v1", "hooks": [...]}`, each hook named, naming the event that triggers
//! it, with its own matcher, a command line as its action (its shell is not documented: POSIX
//! taken elsewhere, none on Windows) and a timeout in seconds. Cortex's hooks are a file of their
//! own, `cortex.json`: for a repository in `.kiro/hooks/`, which git is told to leave out, and for
//! the user in `~/.kiro/hooks/`, which only the command-line agent reads; the editor reads a
//! repository's alone.

use cortex_core::Agent;

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit, Version};
use super::{FoundBy, HooksFile, Setup};

/// How Kiro's hooks file holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "kiro",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Unknown
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::Triggers,
    spelling: Spelling::AsAdapter,
    layout: Layout::Flat,
    keys: Keys::CLAUDE,
    version: Some(Version::Text("v1")),
};

/// Kiro: found by `~/.kiro` or the `kiro-cli` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Kiro,
    found_by: FoundBy {
        folders: &[".kiro"],
        programs: &["kiro-cli"],
    },
    hooks: Some(HooksFile {
        project: Some(".kiro/hooks/cortex.json"),
        user: ".kiro/hooks/cortex.json",
        style: STYLE,
        read_instead: None,
    }),
    told: &[],
};
