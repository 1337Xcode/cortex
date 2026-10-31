//! The daemon: the long-lived process that keeps a repository's index, and its only writer.
//!
//! It takes the repository's run lock, listens for hooks and commands at once (so that requests
//! made while it starts wait for it rather than fail), then takes the store's writer and indexes
//! what changed since the last index. Then it watches the repository and indexes again after each
//! change. In huge mode it parses the files it took inventory of, those that changed first, at
//! `index.crawl_files_per_second` on one thread. It stops when asked, or after
//! `daemon.idle_shutdown_minutes` without a file change, once nothing waits to be parsed. It says
//! when the first index is done and the watcher runs.
//!
//! Errors are `anyhow` errors with context, as a binary's are: this crate is the daemon process's
//! top level, and its errors are reported, never matched.

mod child;
mod global_ignore;
mod host;
mod inbox;
mod instance;
mod listener;
mod server;
pub mod settings;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use cortex_core::{RepoId, UserFolder, canonical_root};
use cortex_index::{Change, DiscoverOptions, Grammars, IndexReport, watch};
use cortex_store::{Writer, WriterOptions};

use self::host::Host;
use self::inbox::Inbox;
use self::instance::Instance;
use self::server::Server;

/// Threads that serve connections. Requests are short; two keep one slow client from delaying
/// another.
const SERVER_THREADS: usize = 2;

/// How long the connections still open get to finish when the daemon stops.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// Runs the daemon for the repository whose root folder is `root`, in any spelling, until it is
/// asked to stop or has been idle long enough. Each line a person running it in the foreground should see
/// (configuration warnings, the ready line) goes to `note`.
///
/// # Errors
///
/// A root that is not a folder, no home folder, another daemon serving the repository, an
/// endpoint that cannot be bound, an
/// unusable configuration, a store that cannot be opened, a watcher that cannot start or stops, a
/// failed index, and a line `note` could not write.
pub fn run(root: &Path, note: &mut dyn FnMut(&str) -> anyhow::Result<()>) -> anyhow::Result<()> {
    // The repository is named by its canonical root, however the caller spelled it, so that the
    // daemon's lock and endpoint are the ones hooks and commands look for.
    let root =
        &canonical_root(root).with_context(|| format!("finding the folder {}", root.display()))?;
    let user = UserFolder::of_current_user().context("finding the user's home folder")?;
    let repo_id = RepoId::of(root);
    let _instance = Instance::acquire(&user, &repo_id)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(SERVER_THREADS)
        .thread_name("cortex-server")
        .enable_all()
        .build()
        .context("starting the server's runtime")?;
    let inbox = Arc::new(Inbox::default());
    let listener = {
        let _inside = runtime.enter();
        listener::bind(&user.run_files(&repo_id)).context("listening for hooks")?
    };
    runtime.spawn(Arc::new(Server::new(Arc::clone(&inbox))).serve(listener));
    let served = index_and_serve(root, (&repo_id, &inbox), note);
    runtime.shutdown_timeout(STOP_GRACE);
    served
}

/// Takes the store's writer, indexes, watches and serves the inbox until asked to stop or idle.
fn index_and_serve(
    root: &Path,
    (repo_id, inbox): (&RepoId, &Arc<Inbox>),
    note: &mut dyn FnMut(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let loaded = settings::load(root)?;
    for warning in &loaded.warnings {
        note(&format!("warning: {warning}"))?;
    }
    let config = loaded.config;
    let grammars = Arc::new(Grammars::load().context("loading the language grammars")?);
    let folder = root.join(".cortex");
    // INVARIANT: single-writer (the store's writer is opened here only, and its lock refuses a
    // second one)
    let writer = Writer::open(&WriterOptions {
        folder: &folder,
        repo_id,
        mmap_bytes: config.mmap_bytes,
        now_ms: host::now_ms(),
    })
    .with_context(|| format!("opening the store in {}", folder.display()))?;
    let global_gitignore = match global_ignore::find(root) {
        Ok(found) => found,
        Err(error) => {
            note(&format!(
                "warning: the global gitignore is not read: {error:#}"
            ))?;
            None
        }
    };
    let discover = DiscoverOptions {
        max_file_bytes: config.max_file_bytes,
        extra_ignores: &config.extra_ignores,
        global_gitignore: global_gitignore.as_deref(),
    };
    let feed = WatcherFeed(Arc::clone(inbox));
    let watcher = watch(root, &discover, move |change| feed.push(change))
        .context("watching the repository")?;
    let mut host = Host::new(root, grammars, (&config, discover), writer);
    let report = host.index(&[])?;
    note(&ready_line(&report, host.waiting()))?;
    host.serve(inbox)?;
    drop(watcher);
    Ok(())
}

/// The watcher's way into the inbox. When the watcher lets go of it, the inbox learns that the
/// watcher stopped.
struct WatcherFeed(Arc<Inbox>);

impl WatcherFeed {
    /// Puts the change in the inbox.
    fn push(&self, change: Change) {
        self.0.push_change(change);
    }
}

impl Drop for WatcherFeed {
    /// Tells the inbox the watcher stopped.
    fn drop(&mut self) {
        self.0.watcher_stopped();
    }
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
