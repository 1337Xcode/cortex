//! Augment's Auggie: its hooks under `hooks` in its settings, Claude Code's nested shape, each a command
//! line run through `cmd.exe` on Windows and `/bin/sh` elsewhere, timeouts in milliseconds; for a
//! repository in `.augment/settings.local.json`, and for the user in `~/.augment/settings.json`.

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit};
use cortex_core::Agent;

use super::{FoundBy, HooksFile, Setup};

/// How Augment's settings hold Cortex's hooks.
const STYLE: Style = Style {
    agent: "augment",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Cmd
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Milliseconds,
    events: Events::Under("hooks"),
    spelling: Spelling::AsAdapter,
    layout: Layout::Grouped,
    keys: Keys::CLAUDE,
    version: None,
};

/// Augment: found by `~/.augment` or the `auggie` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Augment,
    found_by: FoundBy {
        folders: &[".augment"],
        programs: &["auggie"],
    },
    hooks: Some(HooksFile {
        project: Some(".augment/settings.local.json"),
        user: ".augment/settings.json",
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[],
};
