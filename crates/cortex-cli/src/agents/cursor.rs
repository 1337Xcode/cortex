//! Cursor: its hooks in `hooks.json`, `{"version": 1, "hooks": {...}}`, each event a list of
//! handlers each with its own matcher, each a command line, run through PowerShell on Windows and
//! a shell elsewhere (which one is not documented: POSIX taken), timeouts in seconds. A
//! repository's `.cursor/hooks.json` is meant to be committed and a hook names this machine's
//! program, so the hooks go in the user's `~/.cursor/hooks.json` whatever the scope.

use cortex_core::Agent;

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit, Version};
use super::{FoundBy, HooksFile, Setup};

/// How Cursor's hooks file holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "cursor",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Powershell
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::Under("hooks"),
    spelling: Spelling::AsAdapter,
    layout: Layout::Flat,
    keys: Keys::CLAUDE,
    version: Some(Version::Number(1)),
};

/// Cursor: found by `~/.cursor` or its `cursor-agent` or `cursor` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Cursor,
    found_by: FoundBy {
        folders: &[".cursor"],
        programs: &["cursor-agent", "cursor"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".cursor/hooks.json",
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[],
};
