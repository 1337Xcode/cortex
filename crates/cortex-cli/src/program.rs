//! This binary as Claude Code's hooks name it.

use anyhow::{Context as _, anyhow};

/// The path of this binary, which the hooks run, as text.
///
/// # Errors
///
/// The path cannot be found, or is not UTF-8, which a JSON settings file cannot hold.
pub(crate) fn this_program() -> anyhow::Result<String> {
    std::env::current_exe()
        .context("finding this binary")?
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow!("the path of this binary is not UTF-8 text"))
}
