//! What the map shows of the repository's configuration: the limits of the history Cortex reads,
//! said beside every number of commits so that a file whose commits all fell outside them does not
//! read as never committed.

use cortex_core::Config;
use serde::Serialize;

/// The history the co-change miner reads, as the configuration limits it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
// The names are the configuration keys' and the API's, where the shared `max` says what they are.
#[allow(clippy::struct_field_names)]
pub(crate) struct HistoryLimits {
    /// `git.max_commits`: the most commits it reads, newest first.
    pub(crate) max_commits: u64,
    /// `git.max_age_days`: the oldest commit it reads, in days.
    pub(crate) max_age_days: u64,
    /// `git.max_files_per_commit`: a commit that changed more files than this is left out.
    pub(crate) max_files_per_commit: u64,
}

/// The configuration the map's answers carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapSettings {
    /// The history Cortex reads.
    pub(crate) history: HistoryLimits,
}

impl MapSettings {
    /// The map's settings from `config`.
    pub(crate) fn of(config: &Config) -> Self {
        Self {
            history: HistoryLimits {
                max_commits: config.git_max_commits,
                max_age_days: config.git_max_age_days,
                max_files_per_commit: config.git_max_files_per_commit,
            },
        }
    }
}
