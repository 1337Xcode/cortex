//! Devin's command-line agent: its hooks under `hooks` in its config, Claude Code's nested shape, each
//! a command line (its shell is not documented: POSIX taken elsewhere, none on Windows), timeouts in
//! seconds; for a repository in `.devin/config.local.json`, which git leaves out, and for the user
//! in `~/.config/devin/config.json` (`%APPDATA%\devin\config.json` on Windows).

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit};
use cortex_core::Agent;

use super::{FoundBy, HooksFile, Setup};

/// How Devin's config holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "devin",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Unknown
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::Under("hooks"),
    spelling: Spelling::AsAdapter,
    layout: Layout::Grouped,
    keys: Keys::CLAUDE,
    version: None,
};

/// Devin: found by its config folder or the `devin` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Devin,
    found_by: FoundBy {
        folders: &[if cfg!(windows) {
            "AppData/Roaming/devin"
        } else {
            ".config/devin"
        }],
        programs: &["devin"],
    },
    hooks: Some(HooksFile {
        project: Some(".devin/config.local.json"),
        user: if cfg!(windows) {
            "AppData/Roaming/devin/config.json"
        } else {
            ".config/devin/config.json"
        },
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[],
};
