// Configuration loading and management
//
// Loads configuration from environment variables (prefix: CORTEX_) with
// fallback to `.cortex/config.toml` in the repository root. Returns typed
// errors naming the specific missing field.

use std::env;
use std::path::{Path, PathBuf};

use thiserror::Error;

// ---------------------------------------------------------------------------
// Named constants
// ---------------------------------------------------------------------------

/// Maximum depth for BFS graph traversal (callers, callees, blast radius).
pub const MAX_TRAVERSAL_DEPTH: u32 = 5;

/// Maximum number of results returned from a single graph query.
pub const MAX_GRAPH_QUERY_RESULTS: usize = 500;

/// Maximum number of concurrent MCP tool calls allowed via semaphore.
pub const MAX_CONCURRENT_TOOL_CALLS: u32 = 4;

/// Default number of read-only connections in the pool.
pub const DEFAULT_READ_POOL_SIZE: usize = 4;

/// Maximum allowed read pool size.
pub const MAX_READ_POOL_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during configuration loading.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A required configuration field was not provided.
    #[error("missing required configuration field: {field}")]
    MissingField { field: String },

    /// A configuration value could not be parsed to the expected type.
    #[error("invalid value for field '{field}': {reason}")]
    InvalidValue { field: String, reason: String },

    /// The TOML config file could not be read.
    #[error("failed to read config file '{path}': {source}")]
    FileRead {
        path: String,
        source: std::io::Error,
    },

    /// The TOML config file could not be parsed.
    #[error("failed to parse config file '{path}': {source}")]
    FileParse {
        path: String,
        source: toml::de::Error,
    },
}

// ---------------------------------------------------------------------------
// Config struct
// ---------------------------------------------------------------------------

/// Application configuration for Cortex.
#[derive(Debug, Clone)]
pub struct Config {
    /// Root path of the repository being indexed (required).
    pub repo_root: PathBuf,

    /// Directory where Cortex stores its database and artifacts.
    /// Defaults to `{repo_root}/.cortex/`.
    pub data_dir: PathBuf,

    /// Logging level filter (e.g. "info", "debug", "warn").
    /// Defaults to "info".
    pub log_level: String,

    /// Maximum depth for graph traversal queries.
    /// Defaults to MAX_TRAVERSAL_DEPTH (5).
    pub max_traversal_depth: u32,

    /// Maximum number of results from a graph query.
    /// Defaults to MAX_GRAPH_QUERY_RESULTS (500).
    pub max_graph_query_results: usize,

    /// Whether to automatically index on file changes.
    /// Defaults to true.
    pub auto_index: bool,

    /// Whether to check for updates on startup.
    /// Defaults to true.
    pub update_check: bool,

    /// Whether to automatically export the bundle after indexing.
    /// Defaults to true.
    pub auto_bundle_export: bool,

    /// Additional repository paths to include in multi-repo mode.
    /// Defaults to empty.
    pub additional_repos: Vec<PathBuf>,

    /// Whether to enable the visualizer UI HTTP server on serve.
    /// Defaults to false.
    pub ui_enabled: bool,

    /// Number of read-only connections in the database pool.
    /// Defaults to DEFAULT_READ_POOL_SIZE (4), clamped to MAX_READ_POOL_SIZE (16).
    pub pool_size: usize,
}

// ---------------------------------------------------------------------------
// TOML deserialization helper
// ---------------------------------------------------------------------------

/// Intermediate struct for deserializing `.cortex/config.toml`.
#[derive(Debug, serde::Deserialize, Default)]
#[serde(default)]
struct TomlConfig {
    repo_root: Option<String>,
    data_dir: Option<String>,
    log_level: Option<String>,
    max_traversal_depth: Option<u32>,
    max_graph_query_results: Option<usize>,
    auto_index: Option<bool>,
    update_check: Option<bool>,
    auto_bundle_export: Option<bool>,
    additional_repos: Option<Vec<String>>,
    ui_enabled: Option<bool>,
    pool_size: Option<usize>,
}

// ---------------------------------------------------------------------------
// Loading logic
// ---------------------------------------------------------------------------

impl Config {
    /// Load configuration by merging environment variables over TOML file values.
    ///
    /// Resolution order (highest priority first):
    /// 1. Environment variables with `CORTEX_` prefix (e.g. `CORTEX_REPO_ROOT`)
    /// 2. Values from `.cortex/config.toml` relative to `repo_root`
    /// 3. Built-in defaults
    ///
    /// `repo_root` is required and has no default. If it cannot be determined
    /// from either the environment or the TOML file, a `ConfigError::MissingField`
    /// is returned.
    pub fn load() -> Result<Self, ConfigError> {
        // First, try to get repo_root from env to locate the TOML file.
        let env_repo_root = env_var("CORTEX_REPO_ROOT");

        // Attempt to load TOML config if we can locate it.
        let toml_cfg = if let Some(ref root) = env_repo_root {
            load_toml_config(Path::new(root)).ok().unwrap_or_default()
        } else {
            TomlConfig::default()
        };

        // If env didn't provide repo_root, try TOML, then fall back to current directory.
        let repo_root_str = env_repo_root
            .or(toml_cfg.repo_root.clone())
            .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()))
            .ok_or_else(|| ConfigError::MissingField {
                field: "repo_root".to_string(),
            })?;
        let repo_root = PathBuf::from(&repo_root_str);

        // Now that we have repo_root, try loading TOML again if we didn't before.
        let toml_cfg = if env_var("CORTEX_REPO_ROOT").is_some() {
            toml_cfg
        } else {
            load_toml_config(&repo_root).ok().unwrap_or_default()
        };

        // data_dir
        let data_dir = env_var("CORTEX_DATA_DIR")
            .or(toml_cfg.data_dir.clone())
            .map(PathBuf::from)
            .unwrap_or_else(|| repo_root.join(".cortex"));

        // log_level
        let log_level = env_var("CORTEX_LOG_LEVEL")
            .or(toml_cfg.log_level.clone())
            .unwrap_or_else(|| "info".to_string());

        // max_traversal_depth
        let max_traversal_depth = parse_env_or_toml::<u32>(
            "CORTEX_MAX_TRAVERSAL_DEPTH",
            toml_cfg.max_traversal_depth,
            "max_traversal_depth",
        )?
        .unwrap_or(MAX_TRAVERSAL_DEPTH);

        // max_graph_query_results
        let max_graph_query_results = parse_env_or_toml::<usize>(
            "CORTEX_MAX_GRAPH_QUERY_RESULTS",
            toml_cfg.max_graph_query_results,
            "max_graph_query_results",
        )?
        .unwrap_or(MAX_GRAPH_QUERY_RESULTS);

        // auto_index
        let auto_index = parse_env_or_toml::<bool>(
            "CORTEX_AUTO_INDEX",
            toml_cfg.auto_index,
            "auto_index",
        )?
        .unwrap_or(true);

        // update_check
        let update_check = parse_env_or_toml::<bool>(
            "CORTEX_UPDATE_CHECK",
            toml_cfg.update_check,
            "update_check",
        )?
        .unwrap_or(true);

        // auto_bundle_export
        let auto_bundle_export = parse_env_or_toml::<bool>(
            "CORTEX_AUTO_BUNDLE_EXPORT",
            toml_cfg.auto_bundle_export,
            "auto_bundle_export",
        )?
        .unwrap_or(true);

        // additional_repos
        let additional_repos = env_var("CORTEX_ADDITIONAL_REPOS")
            .map(|s| {
                s.split(',')
                    .filter(|p| !p.is_empty())
                    .map(|p| PathBuf::from(p.trim()))
                    .collect()
            })
            .or_else(|| {
                toml_cfg
                    .additional_repos
                    .map(|v| v.into_iter().map(PathBuf::from).collect())
            })
            .unwrap_or_default();

        // ui_enabled
        let ui_enabled = parse_env_or_toml::<bool>(
            "CORTEX_UI_ENABLED",
            toml_cfg.ui_enabled,
            "ui_enabled",
        )?
        .unwrap_or(false);

        // pool_size (default 4, clamped to max 16)
        let pool_size = parse_env_or_toml::<usize>(
            "CORTEX_POOL_SIZE",
            toml_cfg.pool_size,
            "pool_size",
        )?
        .unwrap_or(DEFAULT_READ_POOL_SIZE)
        .min(MAX_READ_POOL_SIZE);

        Ok(Config {
            repo_root,
            data_dir,
            log_level,
            max_traversal_depth,
            max_graph_query_results,
            auto_index,
            update_check,
            auto_bundle_export,
            additional_repos,
            ui_enabled,
            pool_size,
        })
    }

    /// Load configuration from a specific TOML file path, with env var overrides.
    ///
    /// This is useful when the repo root is already known (e.g. from CLI args).
    pub fn load_from_repo(repo_root: &Path) -> Result<Self, ConfigError> {
        let toml_cfg = load_toml_config(repo_root).ok().unwrap_or_default();

        let repo_root_path = env_var("CORTEX_REPO_ROOT")
            .map(PathBuf::from)
            .or_else(|| toml_cfg.repo_root.as_ref().map(PathBuf::from))
            .unwrap_or_else(|| repo_root.to_path_buf());

        // data_dir
        let data_dir = env_var("CORTEX_DATA_DIR")
            .or(toml_cfg.data_dir.clone())
            .map(PathBuf::from)
            .unwrap_or_else(|| repo_root_path.join(".cortex"));

        // log_level
        let log_level = env_var("CORTEX_LOG_LEVEL")
            .or(toml_cfg.log_level.clone())
            .unwrap_or_else(|| "info".to_string());

        // max_traversal_depth
        let max_traversal_depth = parse_env_or_toml::<u32>(
            "CORTEX_MAX_TRAVERSAL_DEPTH",
            toml_cfg.max_traversal_depth,
            "max_traversal_depth",
        )?
        .unwrap_or(MAX_TRAVERSAL_DEPTH);

        // max_graph_query_results
        let max_graph_query_results = parse_env_or_toml::<usize>(
            "CORTEX_MAX_GRAPH_QUERY_RESULTS",
            toml_cfg.max_graph_query_results,
            "max_graph_query_results",
        )?
        .unwrap_or(MAX_GRAPH_QUERY_RESULTS);

        // auto_index
        let auto_index = parse_env_or_toml::<bool>(
            "CORTEX_AUTO_INDEX",
            toml_cfg.auto_index,
            "auto_index",
        )?
        .unwrap_or(true);

        // update_check
        let update_check = parse_env_or_toml::<bool>(
            "CORTEX_UPDATE_CHECK",
            toml_cfg.update_check,
            "update_check",
        )?
        .unwrap_or(true);

        // auto_bundle_export
        let auto_bundle_export = parse_env_or_toml::<bool>(
            "CORTEX_AUTO_BUNDLE_EXPORT",
            toml_cfg.auto_bundle_export,
            "auto_bundle_export",
        )?
        .unwrap_or(true);

        // additional_repos
        let additional_repos = env_var("CORTEX_ADDITIONAL_REPOS")
            .map(|s| {
                s.split(',')
                    .filter(|p| !p.is_empty())
                    .map(|p| PathBuf::from(p.trim()))
                    .collect()
            })
            .or_else(|| {
                toml_cfg
                    .additional_repos
                    .map(|v| v.into_iter().map(PathBuf::from).collect())
            })
            .unwrap_or_default();

        // ui_enabled
        let ui_enabled = parse_env_or_toml::<bool>(
            "CORTEX_UI_ENABLED",
            toml_cfg.ui_enabled,
            "ui_enabled",
        )?
        .unwrap_or(false);

        // pool_size (default 4, clamped to max 16)
        let pool_size = parse_env_or_toml::<usize>(
            "CORTEX_POOL_SIZE",
            toml_cfg.pool_size,
            "pool_size",
        )?
        .unwrap_or(DEFAULT_READ_POOL_SIZE)
        .min(MAX_READ_POOL_SIZE);

        Ok(Config {
            repo_root: repo_root_path,
            data_dir,
            log_level,
            max_traversal_depth,
            max_graph_query_results,
            auto_index,
            update_check,
            auto_bundle_export,
            additional_repos,
            ui_enabled,
            pool_size,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read an environment variable, returning None if unset or empty.
fn env_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

/// Load and parse the TOML config file at `{repo_root}/.cortex/config.toml`.
fn load_toml_config(repo_root: &Path) -> Result<TomlConfig, ConfigError> {
    let config_path = repo_root.join(".cortex").join("config.toml");
    let contents = std::fs::read_to_string(&config_path).map_err(|e| ConfigError::FileRead {
        path: config_path.display().to_string(),
        source: e,
    })?;
    let cfg: TomlConfig =
        toml::from_str(&contents).map_err(|e| ConfigError::FileParse {
            path: config_path.display().to_string(),
            source: e,
        })?;
    Ok(cfg)
}

/// Parse an environment variable as type T, falling back to a TOML value.
/// Returns Ok(None) if neither source provides a value.
fn parse_env_or_toml<T>(
    env_name: &str,
    toml_value: Option<T>,
    field_name: &str,
) -> Result<Option<T>, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    if let Some(val_str) = env_var(env_name) {
        let parsed = val_str.parse::<T>().map_err(|e| ConfigError::InvalidValue {
            field: field_name.to_string(),
            reason: e.to_string(),
        })?;
        Ok(Some(parsed))
    } else {
        Ok(toml_value)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::sync::Mutex;

    // Serialize tests that modify environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Helper to set env vars for a test, returning a guard that clears them on drop.
    struct EnvGuard {
        vars: Vec<String>,
    }

    impl EnvGuard {
        fn new(vars: &[(&str, &str)]) -> Self {
            for (k, v) in vars {
                // SAFETY: Tests are serialized via ENV_LOCK so no concurrent access.
                unsafe { env::set_var(k, v) };
            }
            EnvGuard {
                vars: vars.iter().map(|(k, _)| k.to_string()).collect(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for k in &self.vars {
                // SAFETY: Tests are serialized via ENV_LOCK so no concurrent access.
                unsafe { env::remove_var(k) };
            }
        }
    }

    /// Helper to remove an env var safely in tests (serialized by ENV_LOCK).
    fn remove_env(name: &str) {
        // SAFETY: Tests are serialized via ENV_LOCK so no concurrent access.
        unsafe { env::remove_var(name) };
    }

    #[test]
    fn test_constants() {
        assert_eq!(MAX_TRAVERSAL_DEPTH, 5);
        assert_eq!(MAX_GRAPH_QUERY_RESULTS, 500);
        assert_eq!(MAX_CONCURRENT_TOOL_CALLS, 4);
    }

    #[test]
    fn test_load_from_env_vars() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::new(&[
            ("CORTEX_REPO_ROOT", "/tmp/my-repo"),
            ("CORTEX_DATA_DIR", "/tmp/my-data"),
            ("CORTEX_LOG_LEVEL", "debug"),
            ("CORTEX_MAX_TRAVERSAL_DEPTH", "10"),
            ("CORTEX_MAX_GRAPH_QUERY_RESULTS", "1000"),
            ("CORTEX_AUTO_INDEX", "false"),
            ("CORTEX_UPDATE_CHECK", "false"),
            ("CORTEX_AUTO_BUNDLE_EXPORT", "false"),
            ("CORTEX_ADDITIONAL_REPOS", "/tmp/repo2,/tmp/repo3"),
        ]);

        let cfg = Config::load().expect("should load from env");

        assert_eq!(cfg.repo_root, PathBuf::from("/tmp/my-repo"));
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/my-data"));
        assert_eq!(cfg.log_level, "debug");
        assert_eq!(cfg.max_traversal_depth, 10);
        assert_eq!(cfg.max_graph_query_results, 1000);
        assert!(!cfg.auto_index);
        assert!(!cfg.update_check);
        assert!(!cfg.auto_bundle_export);
        assert_eq!(
            cfg.additional_repos,
            vec![PathBuf::from("/tmp/repo2"), PathBuf::from("/tmp/repo3")]
        );
    }

    #[test]
    fn test_missing_repo_root_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        // Ensure no env vars are set that could provide repo_root.
        remove_env("CORTEX_REPO_ROOT");
        remove_env("CORTEX_DATA_DIR");
        remove_env("CORTEX_LOG_LEVEL");
        remove_env("CORTEX_MAX_TRAVERSAL_DEPTH");
        remove_env("CORTEX_MAX_GRAPH_QUERY_RESULTS");
        remove_env("CORTEX_AUTO_INDEX");
        remove_env("CORTEX_UPDATE_CHECK");
        remove_env("CORTEX_AUTO_BUNDLE_EXPORT");
        remove_env("CORTEX_ADDITIONAL_REPOS");

        // When CORTEX_REPO_ROOT is not set, Config::load() falls back to
        // std::env::current_dir(). This should succeed in any normal environment.
        let result = Config::load();
        assert!(result.is_ok(), "Config::load() should fall back to current_dir when CORTEX_REPO_ROOT is unset");

        let config = result.unwrap();
        // The repo_root should be the current working directory.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(config.repo_root, cwd);
    }

    #[test]
    fn test_defaults_for_optional_fields() {
        let _lock = ENV_LOCK.lock().unwrap();
        // Only set the required field.
        let _guard = EnvGuard::new(&[("CORTEX_REPO_ROOT", "/tmp/defaults-test")]);
        // Clear all optional env vars.
        remove_env("CORTEX_DATA_DIR");
        remove_env("CORTEX_LOG_LEVEL");
        remove_env("CORTEX_MAX_TRAVERSAL_DEPTH");
        remove_env("CORTEX_MAX_GRAPH_QUERY_RESULTS");
        remove_env("CORTEX_AUTO_INDEX");
        remove_env("CORTEX_UPDATE_CHECK");
        remove_env("CORTEX_AUTO_BUNDLE_EXPORT");
        remove_env("CORTEX_ADDITIONAL_REPOS");

        let cfg = Config::load().expect("should load with defaults");

        assert_eq!(cfg.repo_root, PathBuf::from("/tmp/defaults-test"));
        assert_eq!(
            cfg.data_dir,
            PathBuf::from("/tmp/defaults-test/.cortex")
        );
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.max_traversal_depth, MAX_TRAVERSAL_DEPTH);
        assert_eq!(cfg.max_graph_query_results, MAX_GRAPH_QUERY_RESULTS);
        assert!(cfg.auto_index);
        assert!(cfg.update_check);
        assert!(cfg.auto_bundle_export);
        assert!(cfg.additional_repos.is_empty());
    }

    #[test]
    fn test_load_from_toml_file() {
        let _lock = ENV_LOCK.lock().unwrap();

        // Create a temporary directory with a .cortex/config.toml
        let tmp_dir = std::env::temp_dir().join("cortex_config_test_toml");
        let cortex_dir = tmp_dir.join(".cortex");
        fs::create_dir_all(&cortex_dir).unwrap();

        let toml_content = r#"
repo_root = "/tmp/toml-repo"
data_dir = "/tmp/toml-data"
log_level = "warn"
max_traversal_depth = 3
max_graph_query_results = 200
auto_index = false
update_check = false
auto_bundle_export = false
additional_repos = ["/tmp/extra1", "/tmp/extra2"]
"#;
        fs::write(cortex_dir.join("config.toml"), toml_content).unwrap();

        // Clear all env vars so TOML is the only source.
        remove_env("CORTEX_REPO_ROOT");
        remove_env("CORTEX_DATA_DIR");
        remove_env("CORTEX_LOG_LEVEL");
        remove_env("CORTEX_MAX_TRAVERSAL_DEPTH");
        remove_env("CORTEX_MAX_GRAPH_QUERY_RESULTS");
        remove_env("CORTEX_AUTO_INDEX");
        remove_env("CORTEX_UPDATE_CHECK");
        remove_env("CORTEX_AUTO_BUNDLE_EXPORT");
        remove_env("CORTEX_ADDITIONAL_REPOS");

        // Use load_from_repo since we know the repo root.
        let cfg = Config::load_from_repo(&tmp_dir).expect("should load from TOML");

        assert_eq!(cfg.repo_root, PathBuf::from("/tmp/toml-repo"));
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/toml-data"));
        assert_eq!(cfg.log_level, "warn");
        assert_eq!(cfg.max_traversal_depth, 3);
        assert_eq!(cfg.max_graph_query_results, 200);
        assert!(!cfg.auto_index);
        assert!(!cfg.update_check);
        assert!(!cfg.auto_bundle_export);
        assert_eq!(
            cfg.additional_repos,
            vec![PathBuf::from("/tmp/extra1"), PathBuf::from("/tmp/extra2")]
        );

        // Cleanup
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_env_vars_override_toml() {
        let _lock = ENV_LOCK.lock().unwrap();

        // Create a TOML file with some values.
        let tmp_dir = std::env::temp_dir().join("cortex_config_test_override");
        let cortex_dir = tmp_dir.join(".cortex");
        fs::create_dir_all(&cortex_dir).unwrap();

        let toml_content = r#"
log_level = "warn"
max_traversal_depth = 3
auto_index = false
"#;
        fs::write(cortex_dir.join("config.toml"), toml_content).unwrap();

        // Set env vars that should override TOML.
        let _guard = EnvGuard::new(&[
            ("CORTEX_REPO_ROOT", tmp_dir.to_str().unwrap()),
            ("CORTEX_LOG_LEVEL", "trace"),
            ("CORTEX_MAX_TRAVERSAL_DEPTH", "8"),
        ]);
        remove_env("CORTEX_DATA_DIR");
        remove_env("CORTEX_MAX_GRAPH_QUERY_RESULTS");
        remove_env("CORTEX_AUTO_INDEX");
        remove_env("CORTEX_UPDATE_CHECK");
        remove_env("CORTEX_AUTO_BUNDLE_EXPORT");
        remove_env("CORTEX_ADDITIONAL_REPOS");

        let cfg = Config::load().expect("should load with overrides");

        // Env overrides TOML.
        assert_eq!(cfg.log_level, "trace");
        assert_eq!(cfg.max_traversal_depth, 8);
        // TOML value used where env not set.
        assert!(!cfg.auto_index);

        // Cleanup
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_invalid_env_value_returns_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::new(&[
            ("CORTEX_REPO_ROOT", "/tmp/invalid-test"),
            ("CORTEX_MAX_TRAVERSAL_DEPTH", "not_a_number"),
        ]);

        let result = Config::load();
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            ConfigError::InvalidValue { ref field, .. } => {
                assert_eq!(field, "max_traversal_depth");
            }
            _ => panic!("expected InvalidValue error, got: {err}"),
        }
    }
}
