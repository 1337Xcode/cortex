//! Factory's Droid: its hooks file is the hooks object itself, Claude Code's nested shape, each a
//! command line given to a shell (POSIX elsewhere; which one on Windows is not documented),
//! timeouts in seconds. Its repository's hooks file is meant to be committed and a hook names this
//! machine's program, so the hooks go in the user's `~/.factory/hooks.json` whatever the scope.

use super::command::Shell;
use super::settings::{Events, Handler, Style, TimeoutUnit};
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
};

/// Droid: found by `~/.factory` or the `droid` program.
pub(crate) const SETUP: Setup = Setup {
    name: "droid",
    title: "Droid",
    found_by: FoundBy {
        folders: &[".factory"],
        programs: &["droid"],
    },
    hooks: Some(HooksFile {
        project: None,
        user: ".factory/hooks.json",
        style: STYLE,
    }),
    told: &[],
};
