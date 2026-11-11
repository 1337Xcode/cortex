//! Zed's agent panel: no hooks, so Cortex reaches it through its MCP server alone, under
//! `context_servers` in the user's settings, `~/.config/zed/settings.json` (on Linux under
//! `$XDG_CONFIG_HOME` when it names an absolute folder, on Windows `%APPDATA%\Zed\settings.json`).
//! Zed's settings are JSON with comments, and a file holding any is never rewritten: init then
//! says the entry to add. Zed reads its instructions from the user's own `AGENTS.md` in its folder
//! and from one file of a repository's, the first it finds of several that other agents read too:
//! Cortex writes none of them, and init says the lines to add to the first, which Zed alone reads.

use cortex_core::Agent;

use super::mcp::{McpFile, McpPlace, Shape};
use super::rules::mcp_rules;
use super::user_path::UserPath;
use super::{FoundBy, Setup};

/// The user's settings file, where Zed reads it on this system.
const SETTINGS: UserPath = if cfg!(windows) {
    UserPath::home("AppData/Roaming/Zed/settings.json")
} else if cfg!(target_os = "macos") {
    UserPath::home(".config/zed/settings.json")
} else {
    UserPath::config("zed/settings.json")
};

/// What the user is to do for Zed's model to call Cortex's tool, `$personal` naming the
/// instructions file of the user's that Zed alone reads.
macro_rules! told {
    ($personal:literal) => {
        concat!(
            "add these lines to the instructions Zed alone reads, ",
            $personal,
            "; Cortex does not write it, since it is yours, nor a repository's instructions file, \
             which other agents read too, Codex and Claude Code among them, which get Cortex \
             through hooks and have no `cortex_where` tool:\n",
            mcp_rules!()
        )
    };
}

/// What the user is to do for Zed, on this system.
const TOLD: &str = if cfg!(windows) {
    told!("%APPDATA%\\Zed\\AGENTS.md")
} else {
    told!("~/.config/zed/AGENTS.md")
};

/// Zed: found by its configuration folder or the `zed` program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Zed,
    found_by: FoundBy {
        folders: &[if cfg!(windows) {
            "AppData/Roaming/Zed"
        } else {
            ".config/zed"
        }],
        programs: &["zed", "zeditor"],
    },
    hooks: None,
    own: None,
    mcp: Some(McpFile {
        place: McpPlace::Files(&[SETTINGS]),
        servers: &["context_servers"],
        shape: Shape::CommandArgs,
        off: None,
    }),
    told_lines: None,
    told: &[TOLD],
};
