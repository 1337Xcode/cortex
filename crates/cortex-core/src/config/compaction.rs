//! The compaction settings: what Cortex gives an agent just before its context is compacted. Off
//! by default until its evaluation shows it helps.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

use super::{ConfigKey, SWITCH};

/// The settings of what a compaction is given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactionConfig {
    /// `compaction.keep_list`: before Claude Code compacts a session's context, give the
    /// compaction's summary the open task's working set to keep (the files it edited, the files it
    /// was warned of and has not read or edited since, the facts it was given, its prompt). Default
    /// `false`.
    pub is_keep_list: bool,
}

/// The compaction keys, each read and checked here.
pub(super) const KEYS: &[ConfigKey] = &[ConfigKey::new(
    "compaction.keep_list",
    "Before Claude Code compacts a session's context, give its summary the open task's working \
     set to keep.",
    |config, value| {
        config.compaction.is_keep_list = value.as_bool().ok_or(SWITCH)?;
        Ok(())
    },
    |config| toml::Value::Boolean(config.compaction.is_keep_list),
)];
