//! OpenClaw: its hooks are a native plugin's code, so Cortex writes the plugin folder its adapter
//! generates, `cortex/` with its entry script, `package.json` and `openclaw.plugin.json`, into the
//! user's `~/.openclaw/extensions/`, which OpenClaw searches for plugins. A workspace's
//! `.openclaw/extensions/` belongs to OpenClaw's agent workspace, not to a repository, so the
//! user's folder is written whatever the scope. Enabling the plugin and letting its hooks see the
//! conversation are the user's: said, never run or written. Cortex's MCP server goes under
//! `mcp.servers` of the user's `~/.openclaw/openclaw.json`, which OpenClaw reads as JSON5: a file
//! holding comments or anything else plain JSON does not is never rewritten. On Windows,
//! OpenClaw's Gateway runs in WSL and its folders are not documented, so nothing is written there:
//! init says to set it up inside WSL.

use cortex_core::Agent;

use super::mcp::{McpFile, McpPlace, Shape};
use super::own::{OwnFile, OwnFiles, OwnKind, UsersFolder, plugin};
use super::user_path::UserPath;
use super::{FoundBy, Setup};

/// The plugin's files for `program`.
///
/// # Errors
///
/// The adapter writes no plugin.
fn files(program: &str) -> anyhow::Result<Vec<OwnFile>> {
    plugin("openclaw", program)
}

/// What the user is to do for OpenClaw where Cortex writes its plugin.
const TOLD: &[&str] = &[
    "turn Cortex's plugin on with `openclaw plugins enable cortex` (and add `cortex` to \
     `plugins.allow` if you keep that list)",
    "let its hooks see the conversation: set `plugins.entries.cortex.hooks.\
     allowConversationAccess` to true in ~/.openclaw/openclaw.json, without which OpenClaw gives \
     the plugin no prompt to add Cortex's text to",
];

/// What the user is to do for OpenClaw on Windows, where Cortex writes nothing.
const TOLD_ON_WINDOWS: &[&str] = &[
    "set Cortex up for OpenClaw inside WSL, where its Gateway runs (`cortex init --agents \
     openclaw` there): on Windows OpenClaw documents no folder of its own, so Cortex writes \
     nothing for it here",
];

/// OpenClaw: found by its folder or the `openclaw` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::OpenClaw,
    found_by: FoundBy {
        folders: &[".openclaw"],
        programs: &["openclaw"],
    },
    hooks: None,
    own: if cfg!(windows) {
        None
    } else {
        Some(OwnFiles {
            project: None,
            user: UsersFolder::At(UserPath::home(".openclaw/extensions")),
            loads_both: false,
            kind: OwnKind::Plugin,
            hides: &[],
            files,
            after_removal: Some(
                "if you added `plugins.entries.cortex` to ~/.openclaw/openclaw.json, as init said, \
                 it can go now that Cortex's OpenClaw plugin is gone",
            ),
        })
    },
    mcp: if cfg!(windows) {
        None
    } else {
        Some(McpFile {
            place: McpPlace::Files(&[UserPath::home(".openclaw/openclaw.json")]),
            servers: &["mcp", "servers"],
            shape: Shape::CommandArgs,
            off: Some(("enabled", false)),
        })
    },
    told_lines: None,
    told: if cfg!(windows) { TOLD_ON_WINDOWS } else { TOLD },
};
