//! OpenCode: its hooks are a plugin's code, so Cortex writes the plugin its adapter generates,
//! `cortex.js`, which OpenCode loads from a repository's `.opencode/plugins/` and from the user's
//! `plugins/` in its configuration folder (`$XDG_CONFIG_HOME/opencode/`, else
//! `~/.config/opencode/`); it loads both, so init refuses one while the other holds Cortex's. The
//! plugin is written for OpenCode 1.x, whose plugin API it uses; OpenCode 2 runs no 1.x plugin,
//! which init says. Cortex's MCP server goes under `mcp` in the user's `opencode.json` (or an
//! `opencode.jsonc` already there), in the 1.x shape, which OpenCode 2 also reads; a repository's
//! `opencode.json` is meant to be committed, and the entry names this machine's program.

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
    plugin("opencode", program)
}

/// OpenCode: found by its configuration or data folder, or the `opencode` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Opencode,
    found_by: FoundBy {
        folders: &[".config/opencode", ".local/share/opencode"],
        programs: &["opencode"],
    },
    hooks: None,
    own: Some(OwnFiles {
        project: Some(".opencode/plugins"),
        user: UsersFolder::At(UserPath::config("opencode/plugins")),
        loads_both: true,
        kind: OwnKind::Plugin,
        hides: &[],
        files,
        after_removal: None,
    }),
    mcp: Some(McpFile {
        place: McpPlace::Files(&[
            UserPath::config("opencode/opencode.json"),
            UserPath::config("opencode/opencode.jsonc"),
        ]),
        servers: &["mcp"],
        shape: Shape::LocalList,
        off: Some(("enabled", false)),
    }),
    told_lines: None,
    told: &[
        "run OpenCode 1.x: Cortex's plugin is written for its plugin API, and OpenCode 2 runs no \
         1.x plugin (a plugin for 2.x is planned)",
    ],
};
