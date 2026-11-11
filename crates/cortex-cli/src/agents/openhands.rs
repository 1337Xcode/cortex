//! OpenHands: its hooks file is the hooks object itself, Claude Code's nested shape, each a command
//! line it runs through `cmd` on Windows and `sh` elsewhere, timeouts in seconds. Its events are
//! named in snake case (`session_start`) or Pascal case, and a file naming one event both ways is
//! refused whole, so Cortex's are spelled as the user's own are, snake case when there are none;
//! a file that keeps its events under `hooks` has every other key ignored, so Cortex's go there
//! too. A repository's
//! `.openhands/hooks.json` is meant to be committed and a hook names this machine's program, so
//! the hooks go in the user's `~/.openhands/hooks.json` whatever the scope, which OpenHands reads
//! only in a working folder with no hooks file of its own. Its hooks run where its agent server
//! runs: on this machine for its local backend alone.

use cortex_core::Agent;

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit};
use super::{FoundBy, HooksFile, Setup};

/// How OpenHands' hooks file holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "openhands",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Cmd
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::WholeFileOrHooks,
    spelling: Spelling::AsFile,
    layout: Layout::Grouped,
    keys: Keys::CLAUDE,
    version: None,
};

/// OpenHands: found by `~/.openhands` or the `openhands` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::OpenHands,
    found_by: FoundBy {
        folders: &[".openhands"],
        programs: &["openhands"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".openhands/hooks.json",
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[
        "run OpenHands with its local backend, in a folder with no `.openhands/hooks.json` of its \
         own: another backend runs hooks away from this machine, and a repository's hooks file \
         replaces the user's",
    ],
};
