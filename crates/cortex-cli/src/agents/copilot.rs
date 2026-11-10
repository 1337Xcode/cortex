//! GitHub Copilot's agent (Copilot CLI and Visual Studio Code's agent mode): its hooks in any JSON
//! file of a hooks folder, `{"version": 1, "hooks": {...}}`, each event a list of handlers each
//! with its own matcher, each a command line under the key of the shell it is for (`powershell`
//! on Windows, `bash` elsewhere), timeouts in seconds as `timeoutSec`. Cortex's hooks are a file of
//! their own, `cortex.json`: for a repository in `.github/hooks/`, which git is told to leave out,
//! and for the user in `~/.copilot/hooks/`. Copilot reads every file of both folders.

use cortex_core::Agent;

use super::command::Shell;
use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit, Version};
use super::{FoundBy, HooksFile, Setup};

/// How Copilot's hooks file holds Cortex's hooks.
const STYLE: Style = Style {
    agent: "copilot",
    handler: Handler::CommandLine(if cfg!(windows) {
        Shell::Powershell
    } else {
        Shell::Posix
    }),
    timeout: TimeoutUnit::Seconds,
    events: Events::Under("hooks"),
    spelling: Spelling::AsAdapter,
    layout: Layout::Flat,
    keys: Keys {
        command: if cfg!(windows) { "powershell" } else { "bash" },
        timeout: "timeoutSec",
        is_typed: true,
    },
    version: Some(Version::Number(1)),
};

/// Copilot: found by `~/.copilot` or the `copilot` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Copilot,
    found_by: FoundBy {
        folders: &[".copilot"],
        programs: &["copilot"],
    },
    hooks: Some(HooksFile {
        project: Some(".github/hooks/cortex.json"),
        user: ".copilot/hooks/cortex.json",
        style: STYLE,
        read_instead: None,
    }),
    told: &[
        "trust the repository's folder when Copilot asks (or start it with `--trust-folder`): \
         Copilot runs a repository's hooks only in a folder it trusts",
    ],
};
