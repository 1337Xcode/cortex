// Top-level error types for Cortex.
//
// Each module defines its own error enum using thiserror. The top-level
// CortexError enum wraps all module errors for unified propagation.
// anyhow is only used in main.rs and cli/ modules; all other modules
// return typed Result<T, ModuleError>.

use thiserror::Error;

use crate::config::ConfigError;

// ---------------------------------------------------------------------------
// Top-level application error
// ---------------------------------------------------------------------------

/// Unified application error. Every module error can be converted into this
/// via the `From` implementations derived by `#[from]`.
#[derive(Debug, Error)]
pub enum CortexError {
    /// Configuration loading or validation failure.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// Database connection or query failure.
    #[error(transparent)]
    Store(#[from] StoreError),

    /// Schema migration failure.
    #[error(transparent)]
    Migration(#[from] MigrationError),

    /// Indexer pipeline failure.
    #[error(transparent)]
    Index(#[from] IndexError),

    /// Source file parsing failure.
    #[error(transparent)]
    Parse(#[from] ParseError),

    /// File watcher failure.
    #[error(transparent)]
    Watch(#[from] WatchError),

    /// MCP protocol or dispatch failure.
    #[error(transparent)]
    Mcp(#[from] McpError),

    /// Security analysis failure.
    #[error(transparent)]
    Security(#[from] SecurityError),

    /// Memory layer failure.
    #[error(transparent)]
    Memory(#[from] MemoryError),

    /// Bundle export/import failure.
    #[error(transparent)]
    Bundle(#[from] BundleError),

    /// Agent detection/configuration failure.
    #[error(transparent)]
    Agent(#[from] AgentError),
}

// ---------------------------------------------------------------------------
// Per-module error types
// ---------------------------------------------------------------------------

/// Errors from the database store layer.
#[derive(Debug, Error)]
pub enum StoreError {
    /// Failed to open or initialize a database connection.
    #[error("database connection failed: {reason}")]
    ConnectionFailed { reason: String },

    /// A SQL query failed to execute.
    #[error("query failed: {reason}")]
    QueryFailed { reason: String },

    /// A uniqueness or foreign key constraint was violated.
    #[error("constraint violation: {reason}")]
    ConstraintViolation { reason: String },
}

/// Errors from the migration runner.
#[derive(Debug, Error)]
pub enum MigrationError {
    /// A migration SQL file could not be read from disk.
    #[error("failed to read migration file '{file}': {reason}")]
    FileReadFailed { file: String, reason: String },

    /// A migration SQL file failed to execute against SQLite.
    #[error("failed to execute migration '{file}': {reason}")]
    SqlExecutionFailed { file: String, reason: String },
}

/// Errors from the indexer pipeline.
#[derive(Debug, Error)]
#[allow(clippy::enum_variant_names)]
pub enum IndexError {
    /// A source file could not be read from disk.
    #[error("failed to read file '{path}': {reason}")]
    FileReadFailed { path: String, reason: String },

    /// Parsing produced no usable tree for extraction.
    #[error("parse failure for '{path}': {reason}")]
    ParseFailed { path: String, reason: String },

    /// Extraction of nodes/edges from the AST failed.
    #[error("extraction failed for '{path}': {reason}")]
    ExtractionFailed { path: String, reason: String },
}

/// Errors from the source file parser.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The file extension is not mapped to any supported language.
    #[error("unsupported language for extension '.{extension}'")]
    UnsupportedLanguage { extension: String },

    /// The parser could not produce a complete tree.
    #[error("parse failed for '{path}' (partial tree: {partial_tree})")]
    ParseFailed { path: String, partial_tree: bool },
}

/// Errors from the file watcher.
#[derive(Debug, Error)]
pub enum WatchError {
    /// The underlying OS watcher could not be initialized.
    #[error("watcher initialization failed: {reason}")]
    InitFailed { reason: String },

    /// An error occurred while processing a file system event.
    #[error("event processing error: {reason}")]
    EventProcessingFailed { reason: String },
}

/// Errors from the MCP server.
#[derive(Debug, Error)]
pub enum McpError {
    /// A JSON-RPC protocol-level error (malformed request, unknown method).
    #[error("protocol error: {reason}")]
    ProtocolError { reason: String },

    /// A tool dispatch error (invalid arguments, tool not found).
    #[error("dispatch error: {reason}")]
    DispatchError { reason: String },
}

/// Errors from the security analysis module.
#[derive(Debug, Error)]
pub enum SecurityError {
    /// A security analysis pass failed.
    #[error("security analysis failed: {reason}")]
    AnalysisFailed { reason: String },
}

/// Errors from the memory layer.
#[derive(Debug, Error)]
pub enum MemoryError {
    /// An observation operation failed.
    #[error("observation operation failed: {reason}")]
    ObservationFailed { reason: String },

    /// An ADR operation failed.
    #[error("ADR operation failed: {reason}")]
    AdrFailed { reason: String },
}

/// Errors from the bundle export/import layer.
#[derive(Debug, Error)]
pub enum BundleError {
    /// Bundle export failed.
    #[error("bundle export failed: {reason}")]
    ExportFailed { reason: String },

    /// Bundle import failed.
    #[error("bundle import failed: {reason}")]
    ImportFailed { reason: String },

    /// The bundle format version is not compatible with this binary.
    #[error("incompatible bundle version: found {found}, expected {expected}")]
    VersionIncompatible { found: u32, expected: u32 },

    /// The bundle checksum does not match the expected value.
    #[error("checksum mismatch: expected '{expected}', got '{actual}'")]
    ChecksumMismatch { expected: String, actual: String },
}

/// Errors from the agent detection/configuration layer.
#[derive(Debug, Error)]
pub enum AgentError {
    /// Agent detection failed.
    #[error("agent detection failed: {reason}")]
    DetectionFailed { reason: String },

    /// Agent configuration write failed.
    #[error("agent configuration failed: {reason}")]
    ConfigurationFailed { reason: String },

    /// Permission denied writing a config file.
    #[error("Permission denied writing {path}. Required: write access to {path}")]
    PermissionDenied { path: String },

    /// Generated config failed validation when parsed back.
    #[error("Generated config for {agent} failed validation: {reason}")]
    ValidationFailed { agent: String, reason: String },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Display tests - verify error messages render correctly
    // -----------------------------------------------------------------------

    #[test]
    fn test_store_error_display() {
        let err = StoreError::ConnectionFailed {
            reason: "file not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "database connection failed: file not found"
        );

        let err = StoreError::QueryFailed {
            reason: "no such table: nodes".to_string(),
        };
        assert_eq!(err.to_string(), "query failed: no such table: nodes");

        let err = StoreError::ConstraintViolation {
            reason: "UNIQUE constraint failed: nodes.fqn".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "constraint violation: UNIQUE constraint failed: nodes.fqn"
        );
    }

    #[test]
    fn test_migration_error_display() {
        let err = MigrationError::FileReadFailed {
            file: "0001_initial_schema.sql".to_string(),
            reason: "permission denied".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "failed to read migration file '0001_initial_schema.sql': permission denied"
        );

        let err = MigrationError::SqlExecutionFailed {
            file: "0002_security_tables.sql".to_string(),
            reason: "near \"CREAT\": syntax error".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "failed to execute migration '0002_security_tables.sql': near \"CREAT\": syntax error"
        );
    }

    #[test]
    fn test_index_error_display() {
        let err = IndexError::FileReadFailed {
            path: "src/main.rs".to_string(),
            reason: "file not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "failed to read file 'src/main.rs': file not found"
        );

        let err = IndexError::ParseFailed {
            path: "src/lib.rs".to_string(),
            reason: "tree-sitter timeout".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "parse failure for 'src/lib.rs': tree-sitter timeout"
        );

        let err = IndexError::ExtractionFailed {
            path: "src/utils.py".to_string(),
            reason: "no root node".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "extraction failed for 'src/utils.py': no root node"
        );
    }

    #[test]
    fn test_parse_error_display() {
        let err = ParseError::UnsupportedLanguage {
            extension: "xyz".to_string(),
        };
        assert_eq!(err.to_string(), "unsupported language for extension '.xyz'");

        let err = ParseError::ParseFailed {
            path: "src/broken.py".to_string(),
            partial_tree: true,
        };
        assert_eq!(
            err.to_string(),
            "parse failed for 'src/broken.py' (partial tree: true)"
        );

        let err = ParseError::ParseFailed {
            path: "src/empty.ts".to_string(),
            partial_tree: false,
        };
        assert_eq!(
            err.to_string(),
            "parse failed for 'src/empty.ts' (partial tree: false)"
        );
    }

    #[test]
    fn test_watch_error_display() {
        let err = WatchError::InitFailed {
            reason: "inotify limit reached".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "watcher initialization failed: inotify limit reached"
        );

        let err = WatchError::EventProcessingFailed {
            reason: "channel closed".to_string(),
        };
        assert_eq!(err.to_string(), "event processing error: channel closed");
    }

    #[test]
    fn test_mcp_error_display() {
        let err = McpError::ProtocolError {
            reason: "invalid JSON-RPC request".to_string(),
        };
        assert_eq!(err.to_string(), "protocol error: invalid JSON-RPC request");

        let err = McpError::DispatchError {
            reason: "unknown tool: foo_bar".to_string(),
        };
        assert_eq!(err.to_string(), "dispatch error: unknown tool: foo_bar");
    }

    #[test]
    fn test_security_error_display() {
        let err = SecurityError::AnalysisFailed {
            reason: "taint propagation cycle detected".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "security analysis failed: taint propagation cycle detected"
        );
    }

    #[test]
    fn test_memory_error_display() {
        let err = MemoryError::ObservationFailed {
            reason: "node not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "observation operation failed: node not found"
        );

        let err = MemoryError::AdrFailed {
            reason: "duplicate title".to_string(),
        };
        assert_eq!(err.to_string(), "ADR operation failed: duplicate title");
    }

    #[test]
    fn test_bundle_error_display() {
        let err = BundleError::ExportFailed {
            reason: "disk full".to_string(),
        };
        assert_eq!(err.to_string(), "bundle export failed: disk full");

        let err = BundleError::ImportFailed {
            reason: "malformed JSON".to_string(),
        };
        assert_eq!(err.to_string(), "bundle import failed: malformed JSON");

        let err = BundleError::VersionIncompatible {
            found: 2,
            expected: 1,
        };
        assert_eq!(
            err.to_string(),
            "incompatible bundle version: found 2, expected 1"
        );

        let err = BundleError::ChecksumMismatch {
            expected: "abc123".to_string(),
            actual: "def456".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "checksum mismatch: expected 'abc123', got 'def456'"
        );
    }

    #[test]
    fn test_agent_error_display() {
        let err = AgentError::DetectionFailed {
            reason: "home directory not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "agent detection failed: home directory not found"
        );

        let err = AgentError::ConfigurationFailed {
            reason: "permission denied writing config".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "agent configuration failed: permission denied writing config"
        );
    }

    // -----------------------------------------------------------------------
    // Conversion tests - verify From impls for CortexError
    // -----------------------------------------------------------------------

    #[test]
    fn test_store_error_converts_to_cortex_error() {
        let store_err = StoreError::ConnectionFailed {
            reason: "test".to_string(),
        };
        let cortex_err: CortexError = store_err.into();
        assert!(matches!(cortex_err, CortexError::Store(_)));
        assert!(cortex_err.to_string().contains("test"));
    }

    #[test]
    fn test_migration_error_converts_to_cortex_error() {
        let mig_err = MigrationError::FileReadFailed {
            file: "0001.sql".to_string(),
            reason: "not found".to_string(),
        };
        let cortex_err: CortexError = mig_err.into();
        assert!(matches!(cortex_err, CortexError::Migration(_)));
        assert!(cortex_err.to_string().contains("0001.sql"));
    }

    #[test]
    fn test_index_error_converts_to_cortex_error() {
        let idx_err = IndexError::FileReadFailed {
            path: "src/lib.rs".to_string(),
            reason: "io error".to_string(),
        };
        let cortex_err: CortexError = idx_err.into();
        assert!(matches!(cortex_err, CortexError::Index(_)));
        assert!(cortex_err.to_string().contains("src/lib.rs"));
    }

    #[test]
    fn test_parse_error_converts_to_cortex_error() {
        let parse_err = ParseError::UnsupportedLanguage {
            extension: "abc".to_string(),
        };
        let cortex_err: CortexError = parse_err.into();
        assert!(matches!(cortex_err, CortexError::Parse(_)));
        assert!(cortex_err.to_string().contains("abc"));
    }

    #[test]
    fn test_watch_error_converts_to_cortex_error() {
        let watch_err = WatchError::InitFailed {
            reason: "limit".to_string(),
        };
        let cortex_err: CortexError = watch_err.into();
        assert!(matches!(cortex_err, CortexError::Watch(_)));
        assert!(cortex_err.to_string().contains("limit"));
    }

    #[test]
    fn test_mcp_error_converts_to_cortex_error() {
        let mcp_err = McpError::ProtocolError {
            reason: "bad request".to_string(),
        };
        let cortex_err: CortexError = mcp_err.into();
        assert!(matches!(cortex_err, CortexError::Mcp(_)));
        assert!(cortex_err.to_string().contains("bad request"));
    }

    #[test]
    fn test_security_error_converts_to_cortex_error() {
        let sec_err = SecurityError::AnalysisFailed {
            reason: "timeout".to_string(),
        };
        let cortex_err: CortexError = sec_err.into();
        assert!(matches!(cortex_err, CortexError::Security(_)));
        assert!(cortex_err.to_string().contains("timeout"));
    }

    #[test]
    fn test_memory_error_converts_to_cortex_error() {
        let mem_err = MemoryError::ObservationFailed {
            reason: "db locked".to_string(),
        };
        let cortex_err: CortexError = mem_err.into();
        assert!(matches!(cortex_err, CortexError::Memory(_)));
        assert!(cortex_err.to_string().contains("db locked"));
    }

    #[test]
    fn test_bundle_error_converts_to_cortex_error() {
        let bundle_err = BundleError::ChecksumMismatch {
            expected: "aaa".to_string(),
            actual: "bbb".to_string(),
        };
        let cortex_err: CortexError = bundle_err.into();
        assert!(matches!(cortex_err, CortexError::Bundle(_)));
        assert!(cortex_err.to_string().contains("aaa"));
    }

    #[test]
    fn test_agent_error_converts_to_cortex_error() {
        let agent_err = AgentError::DetectionFailed {
            reason: "no agents".to_string(),
        };
        let cortex_err: CortexError = agent_err.into();
        assert!(matches!(cortex_err, CortexError::Agent(_)));
        assert!(cortex_err.to_string().contains("no agents"));
    }

    #[test]
    fn test_config_error_converts_to_cortex_error() {
        let config_err = ConfigError::MissingField {
            field: "repo_root".to_string(),
        };
        let cortex_err: CortexError = config_err.into();
        assert!(matches!(cortex_err, CortexError::Config(_)));
        assert!(cortex_err.to_string().contains("repo_root"));
    }

    // -----------------------------------------------------------------------
    // Propagation test - verify ? operator works with CortexError
    // -----------------------------------------------------------------------

    #[test]
    fn test_question_mark_propagation() {
        fn store_op() -> Result<(), StoreError> {
            Err(StoreError::QueryFailed {
                reason: "test".to_string(),
            })
        }

        fn top_level() -> Result<(), CortexError> {
            store_op()?;
            Ok(())
        }

        let result = top_level();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CortexError::Store(StoreError::QueryFailed { .. })
        ));
    }
}
