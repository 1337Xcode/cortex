//! Charm's Crush: its hooks in its JSON configuration, `crush.json`, under `hooks`, each event a
//! list of handlers each with its own matcher, no `type`, each a command line run by the POSIX
//! shell built into Crush on every system, timeouts in seconds. Crush calls this JSON form
//! deprecated in favour of its `crushrc` script, which it reads beside it, merged; a script is not
//! a file Cortex can merge into and back up as data, so the JSON form is written. Crush reads an
//! event's name in any case and in snake case and joins the lists of every spelling of one event,
//! so Cortex's `PreToolUse` runs beside a user's `pre_tool_use`, each once. A repository's
//! `crush.json` is meant to be committed and a hook names this machine's program, so the hooks go
//! in the user's `~/.config/crush/crush.json` whatever the scope.

use cortex_core::Agent;

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit};
use super::{FoundBy, HooksFile, Setup};

/// How Crush's configuration holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "crush",
    handler: Handler::CommandLine(Shell::Posix),
    timeout: TimeoutUnit::Seconds,
    events: Events::Under("hooks"),
    spelling: Spelling::AsAdapter,
    layout: Layout::Flat,
    keys: Keys {
        command: "command",
        timeout: "timeout",
        is_typed: false,
    },
    version: None,
};

/// Crush: found by its configuration or data folder, or the `crush` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Crush,
    found_by: FoundBy {
        folders: &[
            ".config/crush",
            if cfg!(windows) {
                "AppData/Local/crush"
            } else {
                ".local/share/crush"
            },
        ],
        programs: &["crush"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".config/crush/crush.json",
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[],
};
