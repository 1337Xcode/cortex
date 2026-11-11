//! Kilo Code's command-line agent: its hooks are a plugin's code, so Cortex writes the plugin its
//! adapter generates, `cortex.js`, which Kilo Code loads from a repository's `.kilo/plugin/` and
//! from the user's `plugin/` in its configuration folder (`$XDG_CONFIG_HOME/kilo/`, else
//! `~/.config/kilo/`, on Windows too); it loads both, so init refuses one while the other holds
//! Cortex's. Cortex's MCP server goes under `mcp` in the user's `kilo.json` there, which Kilo Code
//! merges with a `kilo.jsonc` beside it, so a `.jsonc` of the user's, comments and all, is never
//! touched.

use cortex_core::Agent;

use super::mcp::{McpFile, Shape};
use super::own::{OwnFile, OwnFiles, plugin};
use super::user_path::UserPath;
use super::{FoundBy, Setup};

/// The plugin's files for `program`.
///
/// # Errors
///
/// The adapter writes no plugin.
fn files(program: &str) -> anyhow::Result<Vec<OwnFile>> {
    plugin("kilo", program)
}

/// Kilo Code: found by its configuration folder or the `kilo` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Kilo,
    found_by: FoundBy {
        folders: &[".config/kilo"],
        programs: &["kilo"],
    },
    hooks: None,
    own: Some(OwnFiles {
        project: Some(".kilo/plugin"),
        user: Some(UserPath::config("kilo/plugin")),
        loads_both: true,
        what: "plugin",
        files,
        after_removal: None,
    }),
    mcp: Some(McpFile {
        user: &[UserPath::config("kilo/kilo.json")],
        servers: &["mcp"],
        shape: Shape::LocalList,
        off: Some(("enabled", false)),
    }),
    told_lines: None,
    told: &[],
};
