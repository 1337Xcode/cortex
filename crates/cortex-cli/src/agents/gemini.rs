//! Gemini CLI: its hooks under `hooks` in its settings, Claude Code's nested shape, each a command line
//! (POSIX elsewhere; its shell on Windows is not documented), timeouts in milliseconds. Its repository
//! settings are shared through git and a hook names this machine's program, so the hooks go in the
//! user's `~/.gemini/settings.json` whatever the scope.

use super::command::Shell;
use super::settings::{Events, Handler, Style, TimeoutUnit};
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

/// Gemini CLI: found by `~/.gemini` or the `gemini` program.
pub(crate) const SETUP: Setup = Setup {
    name: "gemini",
    title: "Gemini CLI",
    found_by: FoundBy {
        folders: &[".gemini"],
        programs: &["gemini"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".gemini/settings.json",
        style: STYLE,
    }),
    told: &[],
};
