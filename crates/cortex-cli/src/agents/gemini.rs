//! Gemini CLI: its hooks under `hooks` in its settings, Claude Code's nested shape, each a command line
//! (POSIX elsewhere; its shell on Windows is not documented), timeouts in milliseconds. Its repository
//! settings are shared through git and a hook names this machine's program, so the hooks go in the
//! user's `~/.gemini/settings.json` whatever the scope.

use super::command::Shell;
use super::settings::{Events, Handler, Style, TimeoutUnit};
use cortex_core::Agent;

use super::{FoundBy, HooksFile, Setup};

/// How Gemini CLI's settings hold Cortex's hooks.
const STYLE: Style = Style {
    agent: "gemini",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Unknown
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Milliseconds,
    events: Events::UnderHooks,
};

/// Gemini CLI: found by the `gemini` program alone, since `~/.gemini` is also Antigravity's.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Gemini,
    found_by: FoundBy {
        folders: &[],
        programs: &["gemini"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".gemini/settings.json",
        style: STYLE,
        read_instead: None,
    }),
    told: &[],
};
