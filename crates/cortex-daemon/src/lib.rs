//! The daemon: the long-lived process that keeps a repository's index, and its only writer.
//!
//! It takes the repository's run lock, listens for hooks and commands at once (so that requests
//! made while it starts wait for it rather than fail), then takes the store's writer, records the
//! hook events that arrive while the grammars load, and indexes what changed since the last index,
//! recording events between the index's write transactions. Then it watches the repository and
//! indexes again after each change. In huge mode it parses the files it took inventory of, those
//! that changed first, at `index.crawl_files_per_second` on one thread. It stops when asked, or
//! after `daemon.idle_shutdown_minutes` without a request or a file change, once nothing waits to
//! be parsed. It says when the first index is done and the watcher runs. As tasks end it learns
//! from them, and at each stop it reads the transcript the stop names for the tokens each response
//! took.
//!
//! Once it holds the repository it keeps a log, `~/.cortex/logs/daemon-<id>.log`: when it started
//! and stopped and why, the first index, each hook request it handled, and every error it met from
//! then on, the one it stops on included, so that a daemon running detached leaves a record.
//!
//! Errors are `anyhow` errors with context, as a binary's are: this crate is the daemon process's
//! top level, and its errors are reported, never matched.

mod asked;
mod claim;
mod claims;
mod contracts;
mod facts;
mod failed;
mod given;
mod global_ignore;
mod graph;
mod headers;
mod host;
mod inbox;
mod instance;
mod latency;
mod learning;
mod linked;
mod listener;
mod log;
mod meter;
mod miner;
mod noticed;
mod once;
mod paced;
mod pending;
mod place;
mod positions;
mod recapped;
mod recent_errors;
mod retrieval;
mod runs;
mod server;
pub mod settings;
mod signatures;
#[cfg(test)]
mod test_store;
mod told;
mod troubles;
mod upkeep;
mod worktree;

pub use instance::is_running;
pub use upkeep::{Ended, LOG_LIMIT_BYTES, PACE_WAIT, Swept, Upkeep, UpkeepSettings};

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Context;
use cortex_coord::Timing;
use cortex_core::ipc::CORTEX_VERSION;
use cortex_core::{Config, RepoId, UserFolder, canonical_root};
use cortex_index::{Change, DiscoverOptions, IndexReport, watch};
use cortex_learn::{Commands, Recorder};
use cortex_retrieve::GraphSettings;
use cortex_store::{Writer, WriterOptions};

use self::graph::{Snapshot, rebuild, store_reads};
use self::host::Host;
use self::inbox::Inbox;
use self::instance::Instance;
use self::linked::Linked;
use self::meter::Meter;
use self::miner::Miner;
use self::pending::Pending;
use self::retrieval::{EditSettings, FailureSettings, Service, Settings};
use self::server::Server;
use self::signatures::Signatures;
use self::worktree::Worktree;

/// Threads that serve connections. Requests are short; two keep one slow client from delaying
/// another.
const SERVER_THREADS: usize = 2;

/// How long the connections still open get to finish when the daemon stops.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// Runs the daemon for the repository whose root folder is `root`, in any spelling, until it is
/// asked to stop or has been idle long enough. Each line a person running it in the foreground
/// should see (configuration warnings, the ready line) goes to `note`.
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
    let home = std::env::home_dir().context("finding the user's home folder")?;
    let user = UserFolder::in_home(&home);
    let repo_id = RepoId::of(root);
    let _instance = Instance::acquire(&user, &repo_id)?;
    // Started once the repository is held, so that only the daemon serving it writes its log, and
    // dropped before the hold is let go, so that every line is written before another daemon may
    // start.
    let _log = log::start(&user, &repo_id).context("starting the daemon's log")?;
    tracing::info!(
        version = CORTEX_VERSION,
        pid = std::process::id(),
        root = %root.display(),
        "daemon started"
    );
    let served = listen_and_serve(root, (&user, &repo_id, home), note);
    match &served {
        Ok(()) => tracing::info!("daemon stopped"),
        Err(error) => {
            tracing::error!(error = %format!("{error:#}"), "daemon stopped on an error");
        }
    }
    served
}

/// Starts the server's runtime and listens for hooks and commands, then indexes, watches and serves
/// until asked to stop or idle, and gives the connections still open a moment to finish. The user's
/// home folder, `home`, is where the agents keep the transcripts the stops name.
fn listen_and_serve(
    root: &Path,
    (user, repo_id, home): (&UserFolder, &RepoId, PathBuf),
    note: &mut dyn FnMut(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(SERVER_THREADS)
        .thread_name("cortex-server")
        .enable_all()
        .build()
        .context("starting the server's runtime")?;
    let inbox = Arc::new(Inbox::default());
    let shared = Shared {
        linked: Arc::new(Linked::new(root, Arc::clone(&inbox))),
        inbox,
        ..Shared::default()
    };
    let listener = {
        let _inside = runtime.enter();
        listener::bind(&user.run_files(repo_id)).context("listening for hooks")?
    };
    let server = Server::new(
        Arc::clone(&shared.inbox),
        (
            Arc::clone(&shared.snapshot),
            Arc::clone(&shared.worktree),
            Arc::clone(&shared.linked),
            Arc::clone(&shared.signatures),
        ),
        Arc::clone(&shared.service),
    );
    runtime.spawn(Arc::new(server).serve(listener));
    let served = index_and_serve(root, (repo_id, &shared, home), note);
    // However the work ended, before the writer opened included, the server takes no more events
    // in the moment it has left: nothing would record them.
    shared.inbox.close();
    runtime.shutdown_timeout(STOP_GRACE);
    served
}

/// What the server and the writer share: the writer's inbox, the graph, the work tree's
/// uncommitted diff and those of the linked working trees agents work in, the signature changes the
/// writer's indexes found lately, and what answers with text once the first index is done.
#[derive(Debug, Default)]
struct Shared {
    /// The writer's inbox.
    inbox: Arc<Inbox>,
    /// The graph retrieval spreads over.
    snapshot: Arc<Snapshot>,
    /// The work tree's uncommitted diff, which seeds retrievals.
    worktree: Arc<Worktree>,
    /// The linked working trees agents work in, and their own uncommitted diffs.
    linked: Arc<Linked>,
    /// The signature changes the writer's indexes found lately, for an edit's warnings.
    signatures: Arc<Signatures>,
    /// What answers with text.
    service: Arc<OnceLock<Service>>,
}

/// Takes the store's writer, indexes, watches and serves the inbox until asked to stop or idle,
/// reading the transcripts the stops name under the user's home folder, `home`, and keeping
/// `snapshot`'s graph rebuilt from the first index on.
fn index_and_serve(
    root: &Path,
    (repo_id, shared, home): (&RepoId, &Shared, PathBuf),
    note: &mut dyn FnMut(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let (inbox, snapshot, service) = (&shared.inbox, &shared.snapshot, &shared.service);
    let loaded = settings::load(root)?;
    for warning in &loaded.warnings {
        tracing::warn!(warning = %warning, "the configuration");
        note(&format!("warning: {warning}"))?;
    }
    let config = loaded.config;
    inbox.keep_positions_by(Timing::of(&config.coordination));
    // The configuration refuses a pattern that does not compile, so this fails only as it would.
    let commands = Commands::new(&config.test_extra_patterns)?;
    let failure_commands = Arc::new(Commands::new(&config.test_extra_patterns)?);
    inbox.read_failures_with(Arc::clone(&failure_commands));
    let folder = root.join(".cortex");
    // INVARIANT: single-writer (the store's writer is opened here only, and its lock refuses a
    // second one)
    let mut writer = Writer::open(&WriterOptions {
        folder: &folder,
        repo_id,
        mmap_bytes: config.mmap_bytes,
        now_ms: host::now_ms(),
    })
    .with_context(|| format!("opening the store in {}", folder.display()))?;
    inbox.watch_checkpoints(writer.checkpoint_failures());
    // The store opens first, so that hook events arriving while the grammars load are recorded;
    // those that came while it opened are recorded at once.
    let excerpt = config.store_prompt_excerpt;
    let mut recorder = Recorder::new(excerpt, config.holdout_share, commands);
    // The environment is the one the daemon started in: a hook's, which is its agent's.
    let mut pending = Pending::new(Meter::new(&home, &|name| std::env::var_os(name)));
    pending.wait(host::record_waiting(&mut writer, &mut recorder, inbox));
    // From here on, a daemon that cannot go on still records the events it took before it stops.
    let grammars = or_close(
        host::load_grammars((&mut writer, &mut recorder), &mut pending, inbox),
        || close_and_settle(&mut writer, &mut recorder, &mut pending, (inbox, &config)),
    )?;
    let global_gitignore = match global_ignore::find(root) {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "the global gitignore is not read");
            or_close(
                note(&format!(
                    "warning: the global gitignore is not read: {error:#}"
                )),
                || close_and_settle(&mut writer, &mut recorder, &mut pending, (inbox, &config)),
            )?;
            None
        }
    };
    let discover = DiscoverOptions {
        max_file_bytes: config.max_file_bytes,
        extra_ignores: &config.extra_ignores,
        global_gitignore: global_gitignore.as_deref(),
    };
    let feed = WatcherFeed(Arc::clone(inbox));
    let watcher = or_close(
        watch(root, &discover, move |change| feed.push(change)).context("watching the repository"),
        || close_and_settle(&mut writer, &mut recorder, &mut pending, (inbox, &config)),
    )?;
    pending.wait(host::record_waiting(&mut writer, &mut recorder, inbox));
    let mut host = Host::new(
        root,
        Arc::clone(&grammars),
        (&config, discover),
        (
            writer,
            recorder,
            pending,
            (inbox, snapshot, &shared.signatures),
        ),
    );
    let report = or_close(host.index(&[]), || host.close())?;
    inbox.mark_indexed();
    tracing::info!(
        files_written = report.files,
        files_unchanged = report.unchanged,
        mode = report.mode.name(),
        files_waiting = host.waiting(),
        "first index done"
    );
    // Installed once: the daemon serves one repository with one configuration. Installed before
    // the graph's first build, so that a daemon whose status shows a graph answers with text.
    drop(service.set(Service::new(Settings {
        root: root.to_path_buf(),
        folder: folder.clone(),
        mmap_bytes: config.mmap_bytes,
        retrieval: config.retrieval,
        max_file_bytes: config.max_file_bytes,
        holdout_share: config.holdout_share,
        edits: EditSettings {
            grammars,
            parse_timeout: Duration::from_millis(config.parse_timeout_ms),
            warnings: config.warnings,
            link_decay: config.link_decay,
            is_strict_mode: config.is_strict_mode,
        },
        coordination: config.coordination,
        failures: FailureSettings {
            commands: failure_commands,
            is_runtime_grounding: config.hints.is_runtime_grounding,
            is_narrow_tests: config.hints.is_narrow_tests,
            is_redundant_commands: config.hints.is_redundant_commands,
        },
        is_keep_list: config.compaction.is_keep_list,
    })));
    // The graph is built once the first index is done, before which a retrieval would find
    // nothing; told to stop when dropped.
    let rebuilder = rebuild(
        snapshot,
        store_reads((&folder, config.mmap_bytes), GraphSettings::from(&config)),
    );
    // The work tree is looked at once the first index is done, again after every write the graph
    // is told of, which a change of a file comes before, and whenever the miner sees HEAD move;
    // told to stop when dropped.
    snapshot.follow(shared.worktree.asks());
    let looker = worktree::watch(&shared.worktree, root, (&folder, config.mmap_bytes), inbox);
    shared.linked.read_store(&folder, config.mmap_bytes);
    or_close(note(&ready_line(&report, host.waiting())), || host.close())?;
    // The history is mined once the files that count are stored; told to stop when dropped. A
    // miner that cannot start leaves the daemon without co-change, as its status says.
    let miner = Miner::start(root, &config, inbox, &shared.worktree);
    host.serve()?;
    drop(miner);
    shared.linked.stop();
    drop(looker);
    drop(rebuilder);
    drop(watcher);
    Ok(())
}

/// `result`, with `close` run first when it is an error: how the daemon, stopping on an error,
/// still records the events it took.
fn or_close<T>(result: anyhow::Result<T>, close: impl FnOnce()) -> anyhow::Result<T> {
    if result.is_err() {
        close();
    }
    result
}

/// Closes `inbox` to events, records with `recorder` the events it took, learns from the tasks that
/// ended and reads the transcripts named, which wait in `pending`: what a daemon that cannot go on
/// does before it stops.
fn close_and_settle(
    writer: &mut Writer,
    recorder: &mut Recorder,
    pending: &mut Pending,
    (inbox, config): (&Inbox, &Config),
) {
    pending.wait(host::close(writer, recorder, inbox));
    pending.settle((writer, recorder), inbox, config);
}

/// The watcher's way into the inbox. When the watcher lets go of it, the inbox learns that the
/// watcher stopped.
struct WatcherFeed(Arc<Inbox>);

impl WatcherFeed {
    /// Puts the change in the inbox, stamped with the time it arrived.
    fn push(&self, change: Change) {
        self.0.push_change(change, host::now_ms());
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
