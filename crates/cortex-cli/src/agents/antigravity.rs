//! Google Antigravity (its app, its `agy` command-line agent and its editor, which share one hooks
//! file): its `hooks.json` maps each hook's name to its events, so Cortex's are under `cortex`; a
//! tool's events hold matcher groups, Claude Code's nested shape, and the others their handlers
//! straight in the event's list. Each is a command line (its shell is not documented: POSIX taken
//! elsewhere, none on Windows), timeouts in seconds. A repository's `.agents/hooks.json` is meant
//! to be committed and a hook names this machine's program, so the hooks go in the user's
//! `~/.gemini/config/hooks.json` whatever the scope.

use cortex_core::Agent;

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit};
use super::{FoundBy, HooksFile, Setup};

/// How Antigravity's hooks file holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "antigravity",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Unknown
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::Under("cortex"),
    spelling: Spelling::AsAdapter,
    layout: Layout::GroupedWhenMatched,
    keys: Keys::CLAUDE,
    version: None,
};

/// Antigravity: found by the folder of its app, its command-line agent or its editor in
/// `~/.gemini`, or the `agy` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Antigravity,
    found_by: FoundBy {
        folders: &[
            ".gemini/antigravity",
            ".gemini/antigravity-cli",
            ".gemini/antigravity-ide",
        ],
        programs: &["agy"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".gemini/config/hooks.json",
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[],
};
