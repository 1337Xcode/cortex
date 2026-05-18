//! Structured logging via tracing.
//!
//! Initializes a tracing subscriber with env-filter for log level control.
//! Never logs file contents, observation text, or source code.

use tracing_subscriber::EnvFilter;

/// Initialize the tracing subscriber with the given log level filter.
///
/// When `stderr_output` is true (e.g. during `serve`), all logs are written
/// to stderr so that stdout remains clean for JSON-RPC communication.
pub fn init_tracing(log_level: &str, stderr_output: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    if stderr_output {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .init();
    }
}
