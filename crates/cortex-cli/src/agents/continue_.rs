//! Continue: no hooks it fires, so Cortex reaches it through its MCP server alone, and a rules file
//! of Cortex's own tells its model to call `cortex_where`. Continue reads every Markdown file of a
//! repository's `.continue/rules/` and of the user's `~/.continue/rules/` (the second from its
//! source; its documentation names only the first), so the rules file goes to the one the scope
//! names (in both, the model reads the same rules twice, which does no harm); a file of the same
//! name of the user's is never written over. Its MCP servers are in its
//! YAML configuration, which Cortex does not write, or in JSON files of a `mcpServers/` folder, the
//! repository's or the user's (`~/.continue/mcpServers/`, again from its source): Cortex's server
//! goes into a `cortex.json` of its own in the user's, since the entry names this machine's
//! program.

use cortex_core::Agent;

use super::mcp::{McpFile, McpPlace, Shape};
use super::own::{OwnFile, OwnFiles, OwnKind, UsersFolder};
use super::rules::{Opening, rules_file};
use super::user_path::UserPath;
use super::{FoundBy, Setup};

/// Cortex's rules file for Continue, always applied.
///
/// # Errors
///
/// None.
fn files(_: &str) -> anyhow::Result<Vec<OwnFile>> {
    Ok(vec![rules_file(
        "Continue",
        Opening::FrontMatter("name: Cortex\nalwaysApply: true"),
    )])
}

/// Continue: found by its folder or the `cn` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Continue,
    found_by: FoundBy {
        folders: &[".continue"],
        programs: &["cn"],
    },
    hooks: None,
    own: Some(OwnFiles {
        project: Some(".continue/rules"),
        user: UsersFolder::At(UserPath::home(".continue/rules")),
        loads_both: false,
        kind: OwnKind::Rules,
        hides: &[],
        files,
        after_removal: None,
    }),
    mcp: Some(McpFile {
        place: McpPlace::Files(&[UserPath::home(".continue/mcpServers/cortex.json")]),
        servers: &["mcpServers"],
        shape: Shape::CommandArgs,
        off: None,
    }),
    told_lines: None,
    told: &[],
};
