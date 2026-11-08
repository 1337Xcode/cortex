//! Augment's Auggie: its hooks under `hooks` in its settings, Claude Code's nested shape, each a command
//! line run through `cmd.exe` on Windows and `/bin/sh` elsewhere, timeouts in milliseconds; for a
//! repository in `.augment/settings.local.json`, and for the user in `~/.augment/settings.json`.

use super::command::Shell;
use super::settings::{Events, Handler, Style, TimeoutUnit};
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
    events: Events::UnderHooks,
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
    told: &[],
};
