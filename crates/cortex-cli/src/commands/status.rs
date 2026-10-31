//! `cortex status`: whether the repository's daemon runs, whether its index is ready, the scale
//! mode, and how many files are stored, parsed and waiting to be parsed. The store is only read.

use std::path::{Path, PathBuf};

use anyhow::Context;
use cortex_core::{FileFlags, ModeSetting};
use cortex_index::waits_to_be_parsed;
use cortex_store::{MetaKey, Queries, Reader, StoreError, is_writer_running};

use cortex_daemon::settings;

use crate::{output, repository};

/// The arguments of `cortex status`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Prints the repository's status, one fact per line, or that it has no index.
///
/// # Errors
///
/// A root that is not a folder, an unusable configuration, a store that cannot be read, or
/// stdout that cannot be written.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = repository::root(args.root.as_deref())?;
    let loaded = settings::load(&root)?;
    for warning in &loaded.warnings {
        output::note(&format!("warning: {warning}"))?;
    }
    let folder = root.join(".cortex");
    let reader = match Reader::open(&folder, 0) {
        Ok(reader) => reader,
        Err(StoreError::Missing { .. }) => {
            return output::line(&format!(
                "no index in {}; `cortex daemon` builds one",
                root.display()
            ));
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading the store in {}", folder.display()));
        }
    };
    for line in status_lines(&folder, reader.queries(), loaded.config.mode)? {
        output::line(&line)?;
    }
    Ok(())
}

/// The status of the store in `folder` that `queries` reads, with the mode `setting`.
fn status_lines(
    folder: &Path,
    queries: Queries<'_>,
    setting: ModeSetting,
) -> anyhow::Result<Vec<String>> {
    let is_running = is_writer_running(folder)?;
    let index = if queries.meta(MetaKey::CortexVersion)?.is_some() {
        "ready"
    } else if is_running {
        "not ready, an index is under way"
    } else {
        "not ready, the last index did not finish; the next one reads every file again"
    };
    let daemon = if is_running { "running" } else { "not running" };
    let mut mode = queries
        .meta(MetaKey::ScaleMode)?
        .unwrap_or_else(|| "not counted yet".to_owned());
    if let ModeSetting::Pinned(pinned) = setting {
        mode.push_str(&format!(
            "; pinned to {} in the configuration",
            pinned.name()
        ));
    }
    let rows = queries.file_rows()?;
    let parsed = rows
        .iter()
        .filter(|row| row.file.flags.contains(FileFlags::PARSED))
        .count();
    let mut files = format!("{} stored, {parsed} parsed", rows.len());
    let waiting = rows
        .iter()
        .filter(|row| waits_to_be_parsed(&row.file))
        .count();
    if waiting > 0 {
        files.push_str(&format!(", {waiting} waiting to be parsed"));
    }
    Ok(vec![
        format!("daemon: {daemon}"),
        format!("index: {index}"),
        format!("mode: {mode}"),
        format!("files: {files}"),
    ])
}
