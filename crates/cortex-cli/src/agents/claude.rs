//! Claude Code: its hooks in its settings, run as an argument list with no shell.

use cortex_core::Agent;

use super::handler::Keys;
use super::settings::{Events, Handler, Layout, Spelling, Style, TimeoutUnit};

/// How Claude Code's settings hold Cortex's hooks: the exec form, timeouts in seconds, under
/// `hooks`.
pub(crate) const STYLE: Style = Style {
    agent: "claude",
    handler: Handler::Arguments,
    timeout: TimeoutUnit::Seconds,
    events: Events::Under("hooks"),
    spelling: Spelling::AsAdapter,
    layout: Layout::Grouped,
    keys: Keys::CLAUDE,
    version: None,
};

/// Claude Code: found by `~/.claude` or the `claude` program; its hooks in `.claude/settings.local.json`
/// (a file Claude Code does not commit) or the user's `~/.claude/settings.json`.
pub(crate) const SETUP: super::Setup = super::Setup {
    agent: Agent::Claude,
    found_by: super::FoundBy {
        folders: &[".claude"],
        programs: &["claude"],
    },
    hooks: Some(super::HooksFile {
        project: Some(".claude/settings.local.json"),
        user: ".claude/settings.json",
        style: STYLE,
        read_instead: None,
    }),
    own: None,
    mcp: None,
    told_lines: None,
    told: &[],
};
