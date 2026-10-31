//! `cortex daemon`: the long-lived process that keeps a repository's index, and its only writer.
//!
//! It takes the store's writer, so a second daemon on the same repository is refused, and indexes
//! what changed since the last index. Then it watches the repository and indexes again after each
//! change. In huge mode it parses the files it took inventory of, those that changed first, at
//! `index.crawl_files_per_second` on one thread. It stops after `daemon.idle_shutdown_minutes`
//! without a file change, once nothing waits to be parsed. One line on stderr says when the
//! first index is done and the watcher runs.

mod host;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;

use anyhow::{Context, bail};
use cortex_core::RepoId;
use cortex_index::{Change, DiscoverOptions, Grammars, IndexReport, watch};
use cortex_store::{Writer, WriterOptions};

use self::host::Host;
use crate::{output, settings};

/// How many changes may wait for the writer. A change that finds the queue full is dropped: the
/// index the waiting changes start reads the whole tree, the dropped change's files included.
const QUEUE_CHANGES: usize = 10_000;

/// The arguments of `cortex daemon`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// The repository's root folder; the current folder when left out.
    #[arg(long)]
    root: Option<PathBuf>,
}

/// Runs the daemon for the repository until it has been idle long enough.
///
/// # Errors
///
/// A root that is not a folder, an unusable configuration, a store that cannot be opened (another
/// daemon holds it), a watcher that cannot start or stops, and a failed index.
pub(crate) fn run(args: &Args) -> anyhow::Result<()> {
    let root = root_of(args.root.as_deref())?;
    let loaded = settings::load(&root)?;
    for warning in &loaded.warnings {
        output::note(&format!("warning: {warning}"))?;
    }
    let config = loaded.config;
    let grammars = Arc::new(Grammars::load().context("loading the language grammars")?);
    let folder = root.join(".cortex");
    let writer = Writer::open(&WriterOptions {
        folder: &folder,
        repo_id: &RepoId::of(&root),
        mmap_bytes: config.mmap_bytes,
        now_ms: host::now_ms(),
    })
    .with_context(|| format!("opening the store in {}", folder.display()))?;
    let (sender, queue) = mpsc::sync_channel::<Change>(QUEUE_CHANGES);
    let discover = DiscoverOptions {
        max_file_bytes: config.max_file_bytes,
        extra_ignores: &config.extra_ignores,
        global_gitignore: None,
    };
    // A full queue drops the change (see QUEUE_CHANGES); a closed one means the daemon is stopping.
    let watcher = watch(&root, &discover, move |change| {
        drop(sender.try_send(change))
    })
    .context("watching the repository")?;
    let mut host = Host::new(&root, grammars, &config, writer);
    let report = host.index(&[])?;
    output::note(&ready_line(&report, host.waiting()))?;
    host.serve(&queue)?;
    drop(watcher);
    Ok(())
}

/// The repository root `root` names, or the current folder, as an absolute path.
fn root_of(root: Option<&Path>) -> anyhow::Result<PathBuf> {
    let given = match root {
        Some(root) => root.to_owned(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    let absolute = std::path::absolute(&given)
        .with_context(|| format!("making {} absolute", given.display()))?;
    if !absolute.is_dir() {
        bail!("{} is not a folder", absolute.display());
    }
    Ok(absolute)
}

/// The line that says the first index is done: what it wrote, the mode, and in huge mode how many
/// files wait to be parsed.
fn ready_line(report: &IndexReport, waiting: usize) -> String {
    let mut line = format!(
        "daemon ready: mode {}; files: {} written, {} unchanged",
        report.mode.name(),
        report.files,
        report.unchanged
    );
    if waiting > 0 {
        line.push_str(&format!(", {waiting} waiting to be parsed"));
    }
    line
}
