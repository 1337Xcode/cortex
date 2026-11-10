//! Factory's Droid: its hooks file is the hooks object itself, Claude Code's nested shape, each a
//! command line given to a shell (POSIX elsewhere; which one on Windows is not documented),
//! timeouts in seconds. Its repository's hooks file is meant to be committed and a hook names this
//! machine's program, so the hooks go in the user's `~/.factory/hooks.json` whatever the scope.
//! While that file is absent Droid reads hooks from `~/.factory/settings.json`, which init leaves
//! alone; it does not make `hooks.json` over hooks kept there, since Droid would stop reading them.

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit};
use cortex_core::Agent;

use super::{FoundBy, HooksFile, Setup};

/// How Droid's hooks file holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "droid",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Unknown
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::WholeFile,
    spelling: Spelling::AsAdapter,
    layout: Layout::Grouped,
    keys: Keys::CLAUDE,
    version: None,
};

/// Droid: found by `~/.factory` or the `droid` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Droid,
    found_by: FoundBy {
        folders: &[".factory"],
        programs: &["droid"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".factory/hooks.json",
        style: STYLE,
        read_instead: Some(".factory/settings.json"),
    }),
    told: &[],
};
