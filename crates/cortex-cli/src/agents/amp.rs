//! Amp: its hooks are a plugin's code, so Cortex writes the plugin its adapter generates,
//! `cortex.js`, which Amp loads from a repository's `.amp/plugins/` and from the user's `plugins/`
//! in its configuration folder (`$XDG_CONFIG_HOME/amp/`, else `~/.config/amp/`, under the user's
//! profile on Windows too). Of plugins of the same name Amp loads one, the repository's first, so
//! Cortex's in both folders runs once and neither scope refuses the other. Cortex's MCP server
//! goes under the top-level key `amp.mcpServers` of the user's `settings.json` there (or a
//! `settings.jsonc` already there): Amp runs a server of a repository's `.amp/settings.json` only
//! once the user approves it, and that file is meant to be committed.

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
    plugin("amp", program)
}

/// Amp: found by its configuration folder or the `amp` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Amp,
    found_by: FoundBy {
        folders: &[".config/amp"],
        programs: &["amp"],
    },
    hooks: None,
    own: Some(OwnFiles {
        project: Some(".amp/plugins"),
        user: UsersFolder::At(UserPath::config("amp/plugins")),
        loads_both: false,
        kind: OwnKind::Plugin,
        hides: &[],
        files,
        after_removal: None,
    }),
    mcp: Some(McpFile {
        place: McpPlace::Files(&[
            UserPath::config("amp/settings.json"),
            UserPath::config("amp/settings.jsonc"),
        ]),
        servers: &["amp.mcpServers"],
        shape: Shape::CommandArgs,
        off: None,
    }),
    told_lines: None,
    told: &[],
};
