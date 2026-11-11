//! The TRAE IDE (its international edition): its hooks in `hooks.json`, `{"version": 1, "hooks":
//! {...}}` in Claude Code's nested shape, each a command line run through PowerShell on Windows
//! and Bash elsewhere, timeouts in seconds. A repository's `.trae/hooks.json` is meant to be
//! committed and a hook names this machine's program, so the hooks go in the user's
//! `~/.trae/hooks.json` whatever the scope. TRAE runs hooks only once the user has turned them on
//! in its settings.

use cortex_core::Agent;

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit, Version};
use super::{FoundBy, HooksFile, Setup};

/// How TRAE's hooks file holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "trae",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Powershell
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::Under("hooks"),
    spelling: Spelling::AsAdapter,
    layout: Layout::Grouped,
    keys: Keys::CLAUDE,
    version: Some(Version::Number(1)),
};

/// TRAE: found by `~/.trae`.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Trae,
    found_by: FoundBy {
        folders: &[".trae"],
        programs: &[],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".trae/hooks.json",
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[
        "turn hooks on in TRAE's Settings > Hooks, confirming its security warning: TRAE runs no \
         hook until then",
    ],
};
