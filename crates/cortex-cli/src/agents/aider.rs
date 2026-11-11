//! Aider: neither hooks nor MCP, so Cortex writes nothing for it. Its model can suggest a shell
//! command, which Aider runs once the user agrees, and it reads conventions only from files the
//! user names (`--read`, or `read:` in `.aider.conf.yml`), so init says the lines to add to the
//! user's conventions file, telling the model to suggest `cortex where`.

use cortex_core::Agent;

use super::rules::aider_rules;
use super::{FoundBy, Setup};

/// What the user is to do for Aider's model to use Cortex.
const TOLD: &str = concat!(
    "add these lines to the conventions file you give Aider (`--read`, or `read:` in \
     .aider.conf.yml); Cortex writes neither, since they are yours, and Aider reads no file of \
     Cortex's own:\n",
    aider_rules!()
);

/// Aider: found by its program.
pub(crate) const SETUP: Setup = Setup {
    agent: Agent::Aider,
    found_by: FoundBy {
        folders: &[],
        programs: &["aider"],
    },
    hooks: None,
    own: None,
    mcp: None,
    told_lines: None,
    told: &[TOLD],
};
