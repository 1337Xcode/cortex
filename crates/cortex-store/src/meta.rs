//! The keys of the `meta` table: facts about the database itself, and what the self-check last
//! found of the work orders the gate let through.

/// A key in the `meta` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaKey {
    /// The number of the last migration applied.
    SchemaVersion,
    /// The repository's id on this machine.
    RepoId,
    /// The version of Cortex whose rules wrote the index, recorded when an index completes: a file
    /// another version read is read again.
    CortexVersion,
    /// When the database was created, in UTC milliseconds.
    CreatedAt,
    /// The scale mode the index was built for.
    ScaleMode,
    /// What the self-check last found, as JSON.
    SelfCheck,
    /// The last time the confidence model was fitted, kept or not, as JSON.
    CalibrationAttempt,
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
            Self::SelfCheck => "self_check",
            Self::CalibrationAttempt => "calibration_attempt",
        }
    }
}
