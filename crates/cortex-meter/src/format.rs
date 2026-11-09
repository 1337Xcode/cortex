//! How each agent writes its transcript's lines: one format per agent, found by the agent.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cortex_core::Agent;

use crate::claude::Claude;
use crate::codex::Codex;
use crate::response::Response;

/// How one agent writes the lines of its transcripts.
pub trait TranscriptFormat: Sync {
    /// The response `line` reports, without its line ending; `None` for a line that reports no
    /// response, or one that cannot be understood. What a line says that a later line's response
    /// needs (Codex names the model once a turn, not on each response) is kept in `earlier`, which
    /// the reading of one transcript holds from its first line on.
    fn response(&self, line: &[u8], earlier: &mut Earlier) -> Option<Response>;

    /// The folders the agent keeps its transcripts in: under the user's home folder, `home`, or
    /// where the environment, read with `variable`, moves them. A transcript named anywhere else
    /// is not the agent's.
    fn folders(&self, home: &Path, variable: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf>;

    /// Whether a transcript records the model call that compacts its session's conversation.
    /// When it does not, the usage read from it leaves each such call out.
    fn records_compaction(&self) -> bool;
}

/// What earlier lines of a transcript said that a later line's response needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Earlier {
    /// The model the latest line that names one named.
    pub(crate) model: Option<String>,
}

/// The format of `agent`'s transcripts; `None` for an agent whose transcripts are not read yet.
#[must_use]
pub fn format_of(agent: Agent) -> Option<&'static dyn TranscriptFormat> {
    match agent {
        Agent::Claude => Some(&Claude),
        Agent::Codex => Some(&Codex),
        // The other agents write no transcript Cortex reads.
        _ => None,
    }
}

/// Whether `haystack` holds `needle`: a line is searched for a field's name before it is parsed,
/// since most lines hold no response.
pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
