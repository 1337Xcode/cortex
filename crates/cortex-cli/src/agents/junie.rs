//! JetBrains Junie's command-line agent: its hooks under `hooks` in the user's `~/.junie/config.json`,
//! Claude Code's nested shape, each a command line run through `cmd /c` on Windows and `sh -c`
//! elsewhere, timeouts in seconds. Junie ignores a repository's hooks, so the user's file is written
//! whatever the scope.

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit};
use cortex_core::Agent;

use super::{FoundBy, HooksFile, Setup};

/// How Junie's config holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "junie",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Cmd
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::Under("hooks"),
    spelling: Spelling::AsAdapter,
    layout: Layout::Grouped,
    keys: Keys::CLAUDE,
    version: None,
};

/// Junie: found by `~/.junie` or the `junie` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Junie,
    found_by: FoundBy {
        folders: &[".junie"],
        programs: &["junie"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".junie/config.json",
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[],
};
