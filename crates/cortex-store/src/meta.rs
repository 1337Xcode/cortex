//! The keys of the `meta` table: facts about the database itself.

/// A key in the `meta` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaKey {
    /// The number of the last migration applied.
    SchemaVersion,
    /// The repository's id on this machine.
    RepoId,
    /// The version of Cortex that last opened the database for writing.
    CortexVersion,
    /// When the database was created, in UTC milliseconds.
    CreatedAt,
    /// The scale mode the index was built for.
    ScaleMode,
}

impl MetaKey {
    /// The stored key.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::SchemaVersion => "schema_version",
            Self::RepoId => "repo_id",
            Self::CortexVersion => "cortex_version",
            Self::CreatedAt => "created_at",
            Self::ScaleMode => "scale_mode",
        }
    }
}
