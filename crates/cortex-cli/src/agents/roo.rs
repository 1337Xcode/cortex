//! Roo Code's Visual Studio Code extension (no longer developed): no hooks, so Cortex reaches it
//! through its MCP server alone. Its global MCP settings are in the editor's storage for the
//! extension, a path its documentation does not give and a setting can move, and a repository's
//! `.roo/mcp.json` is meant to be committed, so Cortex writes neither: init says the entry to add
//! in Roo's MCP panel. A rules file of Cortex's own in a repository's `.roo/rules/`, every file of
//! which Roo reads, tells the model to call `cortex_where`. Roo reads a repository's `.roorules`
//! and `.clinerules` only while no rules folder, the user's `~/.roo/rules/` included, holds a file,
//! so init makes no first file in `.roo/rules/` beside one of those, and writes none in
//! `~/.roo/rules/`, which would hide them in every repository: at the user's scope it says the
//! lines to add.

use cortex_core::Agent;

use super::mcp::{McpFile, McpPlace, Shape};
use super::own::{OwnFile, OwnFiles, OwnKind, UsersFolder};
use super::rules::{Opening, mcp_rules, rules_file};
use super::{FoundBy, Setup};

/// Cortex's rules file for Roo Code.
///
/// # Errors
///
/// None.
fn files(_: &str) -> anyhow::Result<Vec<OwnFile>> {
    Ok(vec![rules_file("Roo Code", Opening::Comment)])
}

/// What init says at the user's scope, where it writes no rules file.
const FOR_EVERY_REPOSITORY: &str = concat!(
    "for every repository, put these lines in a file of yours in ~/.roo/rules/ (Cortex writes \
     none there, since while that folder holds a file Roo Code reads no repository's .roorules \
     or .clinerules):\n",
    mcp_rules!()
);

/// Roo Code: found by its folder of the user's rules or the `roo` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Roo,
    found_by: FoundBy {
        folders: &[".roo"],
        programs: &["roo"],
    },
    hooks: None,
    own: Some(OwnFiles {
        project: Some(".roo/rules"),
        user: UsersFolder::Told(FOR_EVERY_REPOSITORY),
        loads_both: false,
        kind: OwnKind::Rules,
        hides: &[".roorules", ".clinerules"],
        files,
        after_removal: None,
    }),
    mcp: Some(McpFile {
        place: McpPlace::ByHand(
            "Roo Code's global MCP settings, which its MCP panel opens with Edit Global MCP (their \
             file is in the editor's storage, at a path Roo Code does not document, so Cortex does \
             not write it)",
        ),
        servers: &["mcpServers"],
        shape: Shape::CommandArgs,
        off: None,
    }),
    told_lines: None,
    told: &[],
};
