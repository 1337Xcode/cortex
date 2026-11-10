//! The compaction settings: what Cortex gives an agent just before its context is compacted. Off
//! by default until its evaluation shows it helps.
//!
//! Each key is read and checked here, as the rest of the configuration reads its own; a value the
//! rules cannot use is refused.

/// The settings of what a compaction is given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactionConfig {
    /// `compaction.keep_list`: before Claude Code compacts a session's context, give the
    /// compaction's summary the open task's working set to keep (the files it edited, the files it
    /// was warned of and has not read or edited since, the facts it was given, its prompt). Default
    /// `false`.
    pub is_keep_list: bool,
}

impl CompactionConfig {
    /// Sets the compaction key `key` (written with dots, `compaction.` included) to `value`:
    /// `Some(Ok(()))` when set, `Some(Err(expected))` with what the key accepts when the value is
    /// unusable, `None` when the key is not a compaction key Cortex reads.
    pub(super) fn apply(
        &mut self,
        key: &str,
        value: &toml::Value,
    ) -> Option<Result<(), &'static str>> {
        Some(match key {
            "compaction.keep_list" => value
                .as_bool()
                .ok_or("true or false")
                .map(|is_on| self.is_keep_list = is_on),
            _ => return None,
        })
    }
}
