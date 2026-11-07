//! Qwen Code: its hooks under `hooks` in its settings, Claude Code's nested shape, each a command line
//! for a shell its documentation names only per hook (POSIX elsewhere, not documented by default on
//! Windows), timeouts in seconds. Its repository settings are shared through git and a hook names this
//! machine's program, so the hooks go in the user's `~/.qwen/settings.json` whatever the scope.

use super::command::Shell;
use super::settings::{Events, Handler, Style, TimeoutUnit};
use super::{FoundBy, HooksFile, Setup};

/// How Qwen Code's settings hold Cortex's hooks.
const STYLE: Style = Style {
    agent: "qwen",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Unknown
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::UnderHooks,
};

/// Qwen Code: found by `~/.qwen` or the `qwen` program.
pub(crate) const SETUP: Setup = Setup {
    name: "qwen",
    title: "Qwen Code",
    found_by: FoundBy {
        folders: &[".qwen"],
        programs: &["qwen"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".qwen/settings.json",
        style: STYLE,
    }),
    told: &[],
};
