//! Codex: its hooks in `.codex/hooks.json` in Claude Code's nested shape, each a command line it
//! runs through `cmd /c` on Windows (seen in its runs; its documentation names no shell) and a
//! POSIX shell elsewhere, timeouts in seconds. A hook names this machine's program, and Codex has
//! no file of a repository's that git leaves out, so the hooks go in the user's `~/.codex/hooks.json`
//! whatever the scope. Codex runs a hook only once the user has trusted it in `/hooks`.

use super::command::Shell;
use super::settings::{Events, Handler, Style, TimeoutUnit};
use cortex_core::Agent;

use super::{FoundBy, HooksFile, Setup};

/// How Codex's hooks file holds Cortex's hooks.
pub(crate) const STYLE: Style = Style {
    agent: "codex",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Cmd
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::UnderHooks,
};

/// Codex: found by `~/.codex` or the `codex` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Codex,
    found_by: FoundBy {
        folders: &[".codex"],
        programs: &["codex"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".codex/hooks.json",
        style: STYLE,
    }),
    told: &[
        "trust Cortex's hooks in Codex's `/hooks` (Codex runs no hook it has not been told to trust)",
    ],
};
