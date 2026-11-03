//! How each agent writes its transcript's lines: one format per agent, found by the agent.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cortex_core::Agent;

use crate::claude::Claude;
use crate::response::Response;

/// How one agent writes the lines of its transcripts.
pub trait TranscriptFormat: Sync {
    /// The response `line` reports, without its line ending; `None` for a line that reports no
    /// response, or one that cannot be understood.
    fn response(&self, line: &[u8]) -> Option<Response>;

    /// The folders the agent keeps its transcripts in: under the user's home folder, `home`, or
    /// where the environment, read with `variable`, moves them. A transcript named anywhere else
    /// is not the agent's.
    fn folders(&self, home: &Path, variable: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf>;
}

/// The format of `agent`'s transcripts; `None` for an agent whose transcripts are not read yet.
#[must_use]
pub fn format_of(agent: Agent) -> Option<&'static dyn TranscriptFormat> {
    match agent {
        Agent::Claude => Some(&Claude),
        // Codex's rollout files come with its adapter; the other agents write none Cortex reads.
        _ => None,
    }
}
