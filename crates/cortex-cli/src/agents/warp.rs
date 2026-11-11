//! Warp's agent: no hooks, so Cortex reaches it through its MCP server alone, under `mcpServers` in
//! the user's `~/.warp/.mcp.json`, whose servers Warp starts on its own (a repository's
//! `.warp/.mcp.json` is started only by hand, in each session). Warp reads its rules from Warp
//! Drive, which Cortex cannot write, and from a repository's `WARP.md`, or its `AGENTS.md` while no
//! `WARP.md` is beside it, files the user owns: init says the lines to add where Warp alone reads
//! them. A new `WARP.md` would hide a team's `AGENTS.md` from Warp, and other agents read
//! `AGENTS.md`.

use cortex_core::Agent;

use super::mcp::{McpFile, McpPlace, Shape};
use super::rules::mcp_rules;
use super::user_path::UserPath;
use super::{FoundBy, Setup};

/// What the user is to do for Warp's model to call Cortex's tool.
const TOLD: &str = concat!(
    "add these lines as a Global Rule in Warp Drive (Personal, Rules, Global, or `/add-rule`), \
     which Warp alone reads, or to a repository's WARP.md where it has one; Cortex writes neither, \
     since they are yours, and not AGENTS.md, which other agents read too, Codex among them, \
     which gets Cortex through hooks and has no `cortex_where` tool:\n",
    mcp_rules!()
);

/// Warp: found by its folder or the `warp` and `oz` programs.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Warp,
    found_by: FoundBy {
        folders: &[".warp"],
        programs: &["warp", "oz"],
    },
    hooks: None,
    own: None,
    mcp: Some(McpFile {
        place: McpPlace::Files(&[UserPath::home(".warp/.mcp.json")]),
        servers: &["mcpServers"],
        shape: Shape::CommandArgs,
        off: None,
    }),
    told_lines: None,
    told: &[TOLD],
};
