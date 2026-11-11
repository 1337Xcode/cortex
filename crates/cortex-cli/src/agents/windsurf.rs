//! Windsurf's Cascade agent, which its JetBrains plugin runs (Devin Desktop, Windsurf's editor
//! renamed, starts no Cascade conversation now: its Devin Local agent reads the hooks of Devin's
//! command line, which Cortex's `devin` setup writes):
//! no hooks it can give context with, so Cortex reaches it through its MCP server alone, under
//! `mcpServers` in `mcp_config.json` of the Devin configuration folder (`$XDG_CONFIG_HOME/devin/`,
//! else `~/.config/devin/`; `%APPDATA%\devin\` on Windows), which Devin's command line and Devin
//! Local read too. A rules file of Cortex's own in a repository's `.windsurf/rules/`, which
//! Cascade and Devin Local read, tells the model to call `cortex_where`. The user's global rules
//! are one file of the user's, which Cortex does not write: at the user's scope init says the
//! lines to add there.

use cortex_core::Agent;

use super::mcp::{McpFile, McpPlace, Shape};
use super::own::{OwnFile, OwnFiles, OwnKind, UsersFolder};
use super::rules::{Opening, mcp_rules, rules_file};
use super::user_path::UserPath;
use super::{FoundBy, Setup};

/// Cortex's rules file for Cascade, always on.
///
/// # Errors
///
/// None.
fn files(_: &str) -> anyhow::Result<Vec<OwnFile>> {
    Ok(vec![rules_file(
        "Windsurf",
        Opening::FrontMatter("trigger: always_on"),
    )])
}

/// What init says at the user's scope, where it writes no rules file.
const FOR_EVERY_REPOSITORY: &str = concat!(
    "for every repository, add these lines to Windsurf's global rules (Customizations, Rules, \
     + Global), a file of yours, which Cortex does not write:\n",
    mcp_rules!()
);

/// Windsurf: found by Codeium's folder, which its editor and plugins keep, or the `windsurf`
/// program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Windsurf,
    found_by: FoundBy {
        folders: &[".codeium"],
        programs: &["windsurf"],
    },
    hooks: None,
    own: Some(OwnFiles {
        project: Some(".windsurf/rules"),
        user: UsersFolder::Told(FOR_EVERY_REPOSITORY),
        loads_both: false,
        kind: OwnKind::Rules,
        hides: &[],
        files,
        after_removal: None,
    }),
    mcp: Some(McpFile {
        place: McpPlace::Files(&[if cfg!(windows) {
            UserPath::home("AppData/Roaming/devin/mcp_config.json")
        } else {
            UserPath::config("devin/mcp_config.json")
        }]),
        servers: &["mcpServers"],
        shape: Shape::CommandArgs,
        off: None,
    }),
    told_lines: None,
    told: &[],
};
