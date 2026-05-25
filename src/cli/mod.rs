//! CLI argument parsing and subcommand definitions.
//!
//! Uses clap derive macros to define all subcommands. Each subcommand
//! is dispatched from main.rs after config loading and migration.

pub mod commands;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::version::VERSION;

/// Cortex: Structural codebase intelligence MCP server.
#[derive(Debug, Parser)]
#[command(name = "cortex", version = VERSION, about = "Structural codebase intelligence MCP server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start MCP server with watcher and indexer.
    Serve {
        /// Only expose 5 core tools (ask, search, memory, architecture) to reduce context overhead.
        #[arg(long)]
        smart_tools: bool,
    },

    /// Run full or incremental repository indexing.
    Index,

    /// Parse and extract a single file, print JSON to stdout.
    IndexFile {
        /// Path to the file to parse and extract.
        path: PathBuf,
    },

    /// Auto-detect and configure AI agents.
    Install {
        /// Target platform to configure (auto-detects if omitted).
        /// Supported: claude-code, cursor, windsurf, vscode, kiro, zed, jetbrains, cline,
        /// aider, continue, codex, opencode, openclaw, droid, trae, trae-cn, gemini,
        /// hermes, kimi, pi, copilot, antigravity
        #[arg(long)]
        platform: Option<String>,
    },

    /// Configure Cursor IDE to use Cortex as an MCP server.
    #[command(name = "cursor")]
    Cursor {
        #[command(subcommand)]
        action: CursorCommand,
    },

    /// Configure VS Code (Copilot Chat) to use Cortex as an MCP server.
    #[command(name = "vscode")]
    Vscode {
        #[command(subcommand)]
        action: VscodeCommand,
    },

    /// Configure Kiro IDE to use Cortex as an MCP server.
    #[command(name = "kiro")]
    Kiro {
        #[command(subcommand)]
        action: KiroCommand,
    },

    /// Configure Google Antigravity to use Cortex as an MCP server.
    #[command(name = "antigravity")]
    Antigravity {
        #[command(subcommand)]
        action: AntigravityCommand,
    },

    /// Bundle operations (export/import).
    #[command(subcommand)]
    Bundle(BundleCommand),

    /// Graph query operations.
    #[command(subcommand)]
    Query(QueryCommand),

    /// Show what breaks if you change a function.
    Impact {
        /// Fully qualified name of the function to analyze.
        fqn: String,

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 4)]
        depth: u32,
    },

    /// Explain a function: what it does, what calls it, what it calls, security flags.
    Explain {
        /// Fully qualified name of the function to explain.
        fqn: String,
    },

    /// Compare call graphs between two git branches.
    Diff {
        /// Base branch (e.g. main).
        base: String,

        /// Head branch (e.g. feature-branch).
        head: String,
    },

    /// Run in CI mode: structured output with exit codes for quality gates.
    Ci {
        /// Fail if taint flows are detected.
        #[arg(long)]
        fail_on_taint: bool,

        /// Fail if dead code percentage exceeds this threshold.
        #[arg(long)]
        fail_on_dead_code_above: Option<f64>,

        /// Fail if OWASP patterns are detected.
        #[arg(long)]
        fail_on_owasp: bool,

        /// Output format: "json" or "text".
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Show indexing status.
    Status,

    /// Memory operations.
    #[command(subcommand)]
    Memory(MemoryCommand),

    /// Security operations.
    #[command(subcommand)]
    Security(SecurityCommand),

    /// Configuration management.
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Semantic search management.
    #[command(subcommand)]
    Semantic(SemanticCommand),

    /// Generate a human-readable CORTEX_REPORT.md for the indexed repository.
    Report {
        /// Output path (defaults to CORTEX_REPORT.md in repo root).
        #[arg(long, short)]
        output: Option<PathBuf>,
    },

    /// Open interactive 3D graph visualization in browser.
    Viz {
        /// Export a standalone HTML file instead of starting a server.
        #[arg(long)]
        export: Option<PathBuf>,

        /// Port for the visualization server.
        #[arg(long, default_value_t = 9749)]
        port: u16,
    },

    /// Clone and index a remote repository.
    Clone {
        /// Git URL to clone (HTTPS or SSH).
        url: String,

        /// Local directory to clone into (defaults to repo name).
        #[arg(long)]
        dir: Option<PathBuf>,
    },

    /// Manage git hooks for automatic re-indexing.
    #[command(subcommand)]
    Hook(HookCommand),

    /// Show detected module boundaries (Leiden community detection).
    Modules {
        /// Coupling threshold for community detection (0.0 to 1.0).
        #[arg(long, default_value_t = 0.5)]
        threshold: f64,

        /// Show build system workspace members instead of graph-detected modules.
        #[arg(long)]
        build_system: bool,
    },

    /// Watch for file changes and print re-index events (foreground mode).
    Watch,

    /// Cross-reference call graph with test coverage data (LCOV/gcov).
    Coverage {
        /// Path to LCOV coverage file.
        #[arg(long, default_value = "coverage.lcov")]
        lcov: PathBuf,

        /// Maximum results to show.
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },

    /// Print class inheritance and interface implementation tree.
    Hierarchy {
        /// Class or type FQN to show hierarchy for.
        fqn: String,
    },

    /// Find high-churn, high-complexity code (git + call graph).
    Hotspots {
        /// How many months of git history to analyze.
        #[arg(long, default_value_t = 6)]
        months: u32,

        /// Maximum results to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Multi-repo federation management.
    #[command(subcommand)]
    Federate(FederateCommand),

    /// Ingest documents (markdown, text, CSV, PDF) into the knowledge graph.
    Ingest {
        /// Path to file or directory to ingest.
        path: PathBuf,

        /// File types to include (comma-separated, e.g. "md,txt,pdf").
        #[arg(long, default_value = "md,txt,csv,rst,html,yaml")]
        types: String,
    },
}

/// Cursor subcommands.
#[derive(Debug, Subcommand)]
pub enum CursorCommand {
    /// Configure Cursor to use Cortex as an MCP server.
    Install,
}

/// VS Code subcommands.
#[derive(Debug, Subcommand)]
pub enum VscodeCommand {
    /// Configure VS Code (Copilot Chat) to use Cortex as an MCP server.
    Install,
}

/// Kiro subcommands.
#[derive(Debug, Subcommand)]
pub enum KiroCommand {
    /// Configure Kiro to use Cortex as an MCP server.
    Install,
}

/// Antigravity subcommands.
#[derive(Debug, Subcommand)]
pub enum AntigravityCommand {
    /// Configure Google Antigravity to use Cortex as an MCP server.
    Install,
}

/// Hook subcommands.
#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Install a post-commit git hook for automatic re-indexing.
    Install,

    /// Remove the Cortex git hook.
    Remove,

    /// Show hook status.
    Status,
}

/// Federate subcommands.
#[derive(Debug, Subcommand)]
pub enum FederateCommand {
    /// Add a repository to the federation.
    Add {
        /// Path to the repository to add.
        path: PathBuf,
    },
    /// List all federated repositories.
    List,
    /// Remove a repository from the federation.
    Remove {
        /// Name of the repository to remove.
        name: String,
    },
}

/// Bundle subcommands.
#[derive(Debug, Subcommand)]
pub enum BundleCommand {
    /// Export the graph to a portable JSON bundle.
    Export {
        /// Export format: "json" (default) or "ccg".
        #[arg(long, default_value = "json")]
        format: String,
    },

    /// Import a bundle from a path (defaults to .cortex/cortex.json).
    Import {
        /// Path to the bundle file to import.
        path: Option<PathBuf>,
    },
}

/// Query subcommands.
#[derive(Debug, Subcommand)]
pub enum QueryCommand {
    /// Trace callers of a fully qualified name.
    Callers {
        /// Fully qualified name to trace callers for.
        fqn: String,

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 3)]
        depth: u32,
    },

    /// Trace callees of a fully qualified name.
    Callees {
        /// Fully qualified name to trace callees for.
        fqn: String,

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 3)]
        depth: u32,
    },

    /// Find nodes matching a pattern.
    Find {
        /// Pattern to search for (supports glob matching).
        pattern: String,

        /// Filter by node kind.
        #[arg(long)]
        kind: Option<String>,

        /// Maximum number of results.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },

    /// Show architecture summary.
    Architecture,

    /// Find dead code (nodes with zero callers).
    DeadCode {
        /// Maximum number of results.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },

    /// Compute blast radius for a fully qualified name.
    BlastRadius {
        /// Fully qualified name to compute blast radius for.
        fqn: String,

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 3)]
        depth: u32,
    },

    /// Detect changes since a timestamp.
    Changes {
        /// Unix timestamp to detect changes since.
        #[arg(long)]
        since: u64,
    },
}

/// Memory subcommands.
#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// List observations.
    List,

    /// Prune stale observations.
    Prune {
        /// Only prune stale observations.
        #[arg(long)]
        stale: bool,
    },

    /// Show all stored observations and ADRs.
    Show,
}

/// Security subcommands.
#[derive(Debug, Subcommand)]
pub enum SecurityCommand {
    /// Run security scan (OWASP pattern detection).
    Scan,

    /// Generate SBOM (Software Bill of Materials).
    Sbom,

    /// Check dependencies against OSV.dev for known vulnerabilities.
    Vulns,

    /// Print a human-readable security report to stdout.
    Report,
}

/// Config subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Get a configuration value.
    Get {
        /// Configuration key to retrieve.
        key: String,
    },

    /// Set a configuration value.
    Set {
        /// Configuration key to set.
        key: String,

        /// Value to set.
        value: String,
    },

    /// Reset configuration to defaults.
    Reset,
}

/// Semantic search subcommands.
#[derive(Debug, Subcommand)]
pub enum SemanticCommand {
    /// Enable semantic search (downloads ONNX model).
    Enable,

    /// Disable semantic search.
    Disable,

    /// Show semantic search status.
    Status,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_parse_serve() {
        let cli = Cli::try_parse_from(["cortex", "serve"]).unwrap();
        assert!(matches!(cli.command, Command::Serve { .. }));
    }

    #[test]
    fn test_parse_index() {
        let cli = Cli::try_parse_from(["cortex", "index"]).unwrap();
        assert!(matches!(cli.command, Command::Index));
    }

    #[test]
    fn test_parse_index_file() {
        let cli = Cli::try_parse_from(["cortex", "index-file", "src/main.rs"]).unwrap();
        match cli.command {
            Command::IndexFile { path } => {
                assert_eq!(path, PathBuf::from("src/main.rs"));
            }
            _ => panic!("expected IndexFile command"),
        }
    }

    #[test]
    fn test_parse_install() {
        let cli = Cli::try_parse_from(["cortex", "install"]).unwrap();
        assert!(matches!(cli.command, Command::Install { .. }));
    }

    #[test]
    fn test_parse_bundle_export() {
        let cli = Cli::try_parse_from(["cortex", "bundle", "export"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Bundle(BundleCommand::Export { .. })
        ));
    }

    #[test]
    fn test_parse_bundle_import_default() {
        let cli = Cli::try_parse_from(["cortex", "bundle", "import"]).unwrap();
        match cli.command {
            Command::Bundle(BundleCommand::Import { path }) => {
                assert!(path.is_none());
            }
            _ => panic!("expected Bundle Import command"),
        }
    }

    #[test]
    fn test_parse_bundle_import_with_path() {
        let cli = Cli::try_parse_from(["cortex", "bundle", "import", "/tmp/bundle.json"]).unwrap();
        match cli.command {
            Command::Bundle(BundleCommand::Import { path }) => {
                assert_eq!(path, Some(PathBuf::from("/tmp/bundle.json")));
            }
            _ => panic!("expected Bundle Import command"),
        }
    }

    #[test]
    fn test_parse_query_callers() {
        let cli = Cli::try_parse_from([
            "cortex",
            "query",
            "callers",
            "src/main.rs::main",
            "--depth",
            "5",
        ])
        .unwrap();
        match cli.command {
            Command::Query(QueryCommand::Callers { fqn, depth }) => {
                assert_eq!(fqn, "src/main.rs::main");
                assert_eq!(depth, 5);
            }
            _ => panic!("expected Query Callers command"),
        }
    }

    #[test]
    fn test_parse_query_callers_default_depth() {
        let cli = Cli::try_parse_from(["cortex", "query", "callers", "src/lib.rs::foo"]).unwrap();
        match cli.command {
            Command::Query(QueryCommand::Callers { fqn, depth }) => {
                assert_eq!(fqn, "src/lib.rs::foo");
                assert_eq!(depth, 3);
            }
            _ => panic!("expected Query Callers command"),
        }
    }

    #[test]
    fn test_parse_query_callees() {
        let cli = Cli::try_parse_from([
            "cortex",
            "query",
            "callees",
            "src/main.rs::main",
            "--depth",
            "2",
        ])
        .unwrap();
        match cli.command {
            Command::Query(QueryCommand::Callees { fqn, depth }) => {
                assert_eq!(fqn, "src/main.rs::main");
                assert_eq!(depth, 2);
            }
            _ => panic!("expected Query Callees command"),
        }
    }

    #[test]
    fn test_parse_query_find() {
        let cli = Cli::try_parse_from([
            "cortex", "query", "find", "process*", "--kind", "Function", "--limit", "10",
        ])
        .unwrap();
        match cli.command {
            Command::Query(QueryCommand::Find {
                pattern,
                kind,
                limit,
            }) => {
                assert_eq!(pattern, "process*");
                assert_eq!(kind, Some("Function".to_string()));
                assert_eq!(limit, 10);
            }
            _ => panic!("expected Query Find command"),
        }
    }

    #[test]
    fn test_parse_query_find_defaults() {
        let cli = Cli::try_parse_from(["cortex", "query", "find", "main"]).unwrap();
        match cli.command {
            Command::Query(QueryCommand::Find {
                pattern,
                kind,
                limit,
            }) => {
                assert_eq!(pattern, "main");
                assert!(kind.is_none());
                assert_eq!(limit, 50);
            }
            _ => panic!("expected Query Find command"),
        }
    }

    #[test]
    fn test_parse_query_architecture() {
        let cli = Cli::try_parse_from(["cortex", "query", "architecture"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Query(QueryCommand::Architecture)
        ));
    }

    #[test]
    fn test_parse_query_dead_code() {
        let cli = Cli::try_parse_from(["cortex", "query", "dead-code", "--limit", "100"]).unwrap();
        match cli.command {
            Command::Query(QueryCommand::DeadCode { limit }) => {
                assert_eq!(limit, 100);
            }
            _ => panic!("expected Query DeadCode command"),
        }
    }

    #[test]
    fn test_parse_query_blast_radius() {
        let cli = Cli::try_parse_from([
            "cortex",
            "query",
            "blast-radius",
            "src/db.rs::connect",
            "--depth",
            "4",
        ])
        .unwrap();
        match cli.command {
            Command::Query(QueryCommand::BlastRadius { fqn, depth }) => {
                assert_eq!(fqn, "src/db.rs::connect");
                assert_eq!(depth, 4);
            }
            _ => panic!("expected Query BlastRadius command"),
        }
    }

    #[test]
    fn test_parse_status() {
        let cli = Cli::try_parse_from(["cortex", "status"]).unwrap();
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn test_parse_memory_list() {
        let cli = Cli::try_parse_from(["cortex", "memory", "list"]).unwrap();
        assert!(matches!(cli.command, Command::Memory(MemoryCommand::List)));
    }

    #[test]
    fn test_parse_memory_prune_stale() {
        let cli = Cli::try_parse_from(["cortex", "memory", "prune", "--stale"]).unwrap();
        match cli.command {
            Command::Memory(MemoryCommand::Prune { stale }) => {
                assert!(stale);
            }
            _ => panic!("expected Memory Prune command"),
        }
    }

    #[test]
    fn test_parse_security_scan() {
        let cli = Cli::try_parse_from(["cortex", "security", "scan"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Security(SecurityCommand::Scan)
        ));
    }

    #[test]
    fn test_parse_security_sbom() {
        let cli = Cli::try_parse_from(["cortex", "security", "sbom"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Security(SecurityCommand::Sbom)
        ));
    }

    #[test]
    fn test_parse_security_vulns() {
        let cli = Cli::try_parse_from(["cortex", "security", "vulns"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Security(SecurityCommand::Vulns)
        ));
    }

    #[test]
    fn test_parse_config_get() {
        let cli = Cli::try_parse_from(["cortex", "config", "get", "log_level"]).unwrap();
        match cli.command {
            Command::Config(ConfigCommand::Get { key }) => {
                assert_eq!(key, "log_level");
            }
            _ => panic!("expected Config Get command"),
        }
    }

    #[test]
    fn test_parse_config_set() {
        let cli = Cli::try_parse_from(["cortex", "config", "set", "log_level", "debug"]).unwrap();
        match cli.command {
            Command::Config(ConfigCommand::Set { key, value }) => {
                assert_eq!(key, "log_level");
                assert_eq!(value, "debug");
            }
            _ => panic!("expected Config Set command"),
        }
    }

    #[test]
    fn test_parse_config_reset() {
        let cli = Cli::try_parse_from(["cortex", "config", "reset"]).unwrap();
        assert!(matches!(cli.command, Command::Config(ConfigCommand::Reset)));
    }

    #[test]
    fn test_parse_semantic_enable() {
        let cli = Cli::try_parse_from(["cortex", "semantic", "enable"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Semantic(SemanticCommand::Enable)
        ));
    }

    #[test]
    fn test_parse_semantic_disable() {
        let cli = Cli::try_parse_from(["cortex", "semantic", "disable"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Semantic(SemanticCommand::Disable)
        ));
    }

    #[test]
    fn test_parse_semantic_status() {
        let cli = Cli::try_parse_from(["cortex", "semantic", "status"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Semantic(SemanticCommand::Status)
        ));
    }

    #[test]
    fn test_unknown_subcommand_fails() {
        let result = Cli::try_parse_from(["cortex", "nonexistent"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_subcommand_fails() {
        let result = Cli::try_parse_from(["cortex"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cursor_install() {
        let cli = Cli::try_parse_from(["cortex", "cursor", "install"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Cursor {
                action: CursorCommand::Install
            }
        ));
    }

    #[test]
    fn test_parse_vscode_install() {
        let cli = Cli::try_parse_from(["cortex", "vscode", "install"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Vscode {
                action: VscodeCommand::Install
            }
        ));
    }

    #[test]
    fn test_parse_kiro_install() {
        let cli = Cli::try_parse_from(["cortex", "kiro", "install"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Kiro {
                action: KiroCommand::Install
            }
        ));
    }

    #[test]
    fn test_parse_antigravity_install() {
        let cli = Cli::try_parse_from(["cortex", "antigravity", "install"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Antigravity {
                action: AntigravityCommand::Install
            }
        ));
    }

    #[test]
    fn test_parse_install_with_platform() {
        let cli = Cli::try_parse_from(["cortex", "install", "--platform", "gemini"]).unwrap();
        match cli.command {
            Command::Install { platform } => {
                assert_eq!(platform, Some("gemini".to_string()));
            }
            _ => panic!("expected Install command"),
        }
    }
}
