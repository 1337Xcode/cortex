//! Build version and format version constants.
//!
//! Used by CLI --version and bundle format_version.

/// The current version of the Cortex binary, sourced from Cargo.toml at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
