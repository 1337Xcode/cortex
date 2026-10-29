//! `cortex version`: prints the binary's name and version.

use crate::output;

/// The version this binary was built as, from the workspace manifest.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Prints `cortex <version>` on one line.
///
/// # Errors
///
/// Fails only when stdout cannot be written, for example a closed pipe.
pub(crate) fn run() -> anyhow::Result<()> {
    output::line(&format!("cortex {VERSION}"))
}
