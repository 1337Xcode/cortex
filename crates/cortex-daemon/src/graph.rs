//! The graph retrieval spreads over, held in memory and rebuilt whole from the store after a write
//! changes what it holds: an index, a stored mining, a task's learning.
//!
//! A request takes the graph as it stands and never waits for a rebuild. The rebuilds run on a
//! thread of their own, each built from one read transaction, so that it sees the store as one
//! moment left it, and swapped in whole. However often the store changes, a rebuild starts at least
//! half a second after the one before, and at least four times as long after it as that one took,
//! so that rebuilding a large graph takes at most a quarter of one core. A rebuild that fails, or
//! panics, is counted, kept out of the graph, and tried again, a second later after one failure,
//! twice as long after each further one in a row, at most a minute; the status says how many
//! failed. A stop is taken at once, between rebuilds or while waiting for the next.
//!
//! Co-change links are taken only from a complete mining. The miner takes its mark out of the store
//! before it writes and puts it back once done, and the host holds a [`MiningWrite`] while it
//! writes. A rebuild that finds no mark while a write is under way is let go, the graph standing,
//! since the end of the write asks for another; one that finds no mark otherwise (no mining yet, or
//! one cut short), or that would be the first graph, is built without co-change links.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cortex_core::ipc::GraphReport;
use cortex_retrieve::{Graph, GraphSettings, RetrieveError, read_graph};
use cortex_store::{GitStateKey, NodeId, Reader};

/// The least time between the starts of two rebuilds: writes that come closer together share one.
const REBUILD_EVERY: Duration = Duration::from_millis(500);

/// How many times as long as a rebuild took the next waits, at least, from its start: rebuilding
/// then takes at most a quarter of the thread's time.
const REBUILD_SPACING: u32 = 4;

/// The longest a rebuild that failed waits before it is tried again.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

/// A graph, with whether it holds the co-change links of a complete mining.
pub(crate) type Read = (Graph<NodeId>, bool);

/// The graph as it stands, and what the thread that rebuilds it waits for.
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    /// The graph, once one is built, and how the rebuilds went.
    held: Mutex<Held>,
    /// What the rebuilding thread waits for.
    wanted: Mutex<Wanted>,
    /// Wakes the rebuilding thread when what it waits for changes.
    wake: Condvar,
}

/// The graph, once one is built, and how the rebuilds since the daemon started went.
#[derive(Debug, Default)]
struct Held {
    /// The latest graph built.
    graph: Option<Built>,
    /// How many graphs were built.
    builds: u64,
    /// How many rebuilds failed.
    failed: u64,
}

/// One graph built, and how.
#[derive(Debug, Clone)]
struct Built {
    /// The graph.
    graph: Arc<Graph<NodeId>>,
    /// Whether it holds the co-change links of a complete mining.
    has_cochange: bool,
    /// How long the rebuild took, in milliseconds.
    took_ms: u64,
}

/// What the rebuilding thread waits for.
#[derive(Debug, Default)]
struct Wanted {
    /// The store changed since the latest rebuild began, or the latest failed.
    is_changed: bool,
    /// A mining is being written.
    is_mining: bool,
    /// The thread is to stop.
    is_stopping: bool,
}

/// How the latest rebuild went, which paces the next.
#[derive(Debug, Clone, Copy)]
struct Last {
    /// When it started.
    started: Instant,
    /// How long it took.
    took: Duration,
    /// How many rebuilds in a row, it included, failed.
    failures: u32,
}

/// What the host holds while it writes a mining: the snapshot knows the write is under way, and
/// is asked to rebuild when it ends, however it went.
#[derive(Debug)]
pub(crate) struct MiningWrite<'a> {
    /// The snapshot told.
    snapshot: &'a Snapshot,
}

impl Drop for MiningWrite<'_> {
    /// The write is over: the next rebuild takes what it left.
    fn drop(&mut self) {
        let mut wanted = self.snapshot.wanted();
        wanted.is_mining = false;
        wanted.is_changed = true;
        drop(wanted);
        self.snapshot.wake.notify_all();
    }
}

impl Snapshot {
    /// The graph as it stands, once one is built: requests take it and never wait for a rebuild.
    pub(crate) fn graph(&self) -> Option<Arc<Graph<NodeId>>> {
        self.held()
            .graph
            .as_ref()
            .map(|built| Arc::clone(&built.graph))
    }

    /// What the daemon's status says of the graph: none before the first rebuild ended.
    pub(crate) fn report(&self) -> Option<GraphReport> {
        let held = self.held();
        if held.builds == 0 && held.failed == 0 {
            return None;
        }
        let built = held.graph.as_ref();
        Some(GraphReport {
            nodes: built.map_or(0, |built| {
                u64::try_from(built.graph.len()).unwrap_or(u64::MAX)
            }),
            links: built.map_or(0, |built| {
                u64::try_from(built.graph.link_count()).unwrap_or(u64::MAX)
            }),
            has_cochange: built.is_some_and(|built| built.has_cochange),
            builds: held.builds,
            build_ms: built.map_or(0, |built| built.took_ms),
            failed_builds: held.failed,
        })
    }

    /// Notes that the store changed, so that the graph is rebuilt.
    pub(crate) fn changed(&self) {
        self.wanted().is_changed = true;
        self.wake.notify_all();
    }

    /// Notes that a mining's write begins, until what this returns is dropped.
    pub(crate) fn mining_write(&self) -> MiningWrite<'_> {
        self.wanted().is_mining = true;
        MiningWrite { snapshot: self }
    }

    /// The graph and the rebuilds' counts, locked.
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the rebuilding thread waits for, locked.
    fn wanted(&self) -> MutexGuard<'_, Wanted> {
        self.wanted.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the change waiting, as the rebuilding thread does: whether the store changed since
    /// the last time. For the tests of what tells the snapshot.
    #[cfg(test)]
    pub(crate) fn take_change(&self) -> bool {
        std::mem::take(&mut self.wanted().is_changed)
    }

    /// Whether a mining's write is under way, as the snapshot was told. For tests.
    #[cfg(test)]
    pub(crate) fn is_mining(&self) -> bool {
        self.wanted().is_mining
    }

    /// Tells the rebuilding thread to stop.
    fn stop(&self) {
        self.wanted().is_stopping = true;
        self.wake.notify_all();
    }

    /// Waits until the store changed and `not_before` has come, or the thread is to stop, and takes
    /// the change: `false` to stop. A stop is taken at once, however long the wait still had.
    fn wait_for_change(&self, not_before: Option<Instant>) -> bool {
        let mut wanted = self.wanted();
        loop {
            if wanted.is_stopping {
                return false;
            }
            if !wanted.is_changed {
                wanted = self
                    .wake
                    .wait(wanted)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            let now = Instant::now();
            match not_before.filter(|at| *at > now) {
                Some(at) => {
                    wanted = self
                        .wake
                        .wait_timeout(wanted, at - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                None => {
                    wanted.is_changed = false;
                    return true;
                }
            }
        }
    }

    /// Takes in `read`, a graph read from the store, unless a mining is being written and the graph
    /// standing came from before it: a read that finds the mark out then holds part of the write.
    fn take(&self, (graph, has_cochange): Read, took: Duration) {
        let is_mining = self.wanted().is_mining;
        let took_ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX);
        let (nodes, links) = (graph.len(), graph.link_count());
        {
            let mut held = self.held();
            if !has_cochange && is_mining && held.graph.is_some() {
                drop(held);
                tracing::debug!("a mining is being written: the graph stands");
                return;
            }
            held.builds += 1;
            held.graph = Some(Built {
                graph: Arc::new(graph),
                has_cochange,
                took_ms,
            });
        }
        tracing::info!(nodes, links, has_cochange, took_ms, "graph rebuilt");
    }

    /// Counts a rebuild that failed, and asks for another.
    fn fail(&self, why: &str) {
        self.held().failed += 1;
        self.wanted().is_changed = true;
        tracing::warn!(error = %why, "the graph is not rebuilt");
    }
}

/// The thread that rebuilds a snapshot, stopped and waited for when dropped.
#[derive(Debug)]
pub(crate) struct Rebuilder {
    /// The snapshot it rebuilds.
    snapshot: Arc<Snapshot>,
    /// The thread, when it started.
    thread: Option<JoinHandle<()>>,
}

impl Rebuilder {
    /// Starts rebuilding `snapshot` with what `read` reads: at once, then after every change. A
    /// thread that cannot start is logged, and the daemon retrieves nothing.
    pub(crate) fn start(
        snapshot: &Arc<Snapshot>,
        read: impl FnMut() -> Result<Read, String> + Send + 'static,
    ) -> Self {
        let shared = Arc::clone(snapshot);
        snapshot.changed();
        let thread = std::thread::Builder::new()
            .name("cortex-graph".to_owned())
            .spawn(move || rebuild_until_stopped(&shared, read));
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                tracing::warn!(error = %error, "the graph's thread did not start");
                None
            }
        };
        Self {
            snapshot: Arc::clone(snapshot),
            thread,
        }
    }
}

impl Drop for Rebuilder {
    /// Stops the thread and waits for it: a rebuild under way finishes first.
    fn drop(&mut self) {
        self.snapshot.stop();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::warn!("the graph's thread panicked");
        }
    }
}

/// What reads the graph from the store in the `.cortex` folder `folder`, opened with `mmap_bytes`
/// of memory map, weighed by `settings`: the store is opened at the first read, and again after a
/// read that could not open it.
pub(crate) fn store_reads(
    (folder, mmap_bytes): (&Path, u64),
    settings: GraphSettings,
) -> impl FnMut() -> Result<Read, String> + Send + 'static + use<> {
    let folder: PathBuf = folder.to_path_buf();
    let mut reader: Option<Reader> = None;
    move || {
        let opened = match reader.take() {
            Some(opened) => opened,
            None => Reader::open(&folder, mmap_bytes)
                .map_err(|error| format!("opening the store to read the graph: {error:#}"))?,
        };
        let read = read_whole(&opened, &settings).map_err(|error| format!("{error:#}"));
        reader = Some(opened);
        read
    }
}

/// The whole graph the store `reader` reads holds, weighed by `settings`, in one read transaction:
/// with its co-change links only when the miner's mark is there.
fn read_whole(reader: &Reader, settings: &GraphSettings) -> Result<Read, RetrieveError> {
    reader.consistently(|queries| -> Result<_, RetrieveError> {
        let has_cochange = queries.git_state(GitStateKey::LastCommit)?.is_some();
        let mut weighed = *settings;
        if !has_cochange {
            weighed.type_weights.cochange = 0.0;
        }
        Ok((read_graph(queries, &weighed)?, has_cochange))
    })
}

/// Rebuilds `snapshot` with what `read` reads after each change until told to stop, each start as
/// [`next_start`] paces it. A read that fails or panics is counted and tried again.
fn rebuild_until_stopped(snapshot: &Snapshot, mut read: impl FnMut() -> Result<Read, String>) {
    let mut last: Option<Last> = None;
    while snapshot.wait_for_change(last.map(next_start)) {
        let started = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(&mut read))
            .unwrap_or_else(|_| Err("reading the graph panicked".to_owned()));
        let took = started.elapsed();
        let failures = match outcome {
            Ok(graph) => {
                snapshot.take(graph, took);
                0
            }
            Err(why) => {
                snapshot.fail(&why);
                last.map_or(0, |last| last.failures).saturating_add(1)
            }
        };
        last = Some(Last {
            started,
            took,
            failures,
        });
    }
}

/// When the rebuild after `last` may start: at least [`REBUILD_EVERY`] after it, at least
/// [`REBUILD_SPACING`] times as long as it took, and after failures in a row a second, twice as
/// long for each further one, at most [`MAX_RETRY_WAIT`].
fn next_start(last: Last) -> Instant {
    let paced = REBUILD_EVERY.max(last.took.saturating_mul(REBUILD_SPACING));
    let retry = match last.failures {
        0 => Duration::ZERO,
        failures => REBUILD_EVERY
            .saturating_mul(2_u32.saturating_pow(failures))
            .min(MAX_RETRY_WAIT),
    };
    last.started + paced.max(retry)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use cortex_core::ipc::GraphReport;
    use cortex_core::{Config, EdgeType, FileFlags, RepoId, RepoPath};
    use cortex_store::{
        EdgeRecord, FileIndex, FileRecord, GitStateKey, NodeId, Writer, WriterOptions,
    };

    use super::*;

    /// Any failure fails the test.
    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// How long a test waits for a rebuild it expects.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// The store of the repository at `root`.
    fn writer(root: &Path) -> TestResult<Writer> {
        Ok(Writer::open(&WriterOptions {
            folder: &root.join(".cortex"),
            repo_id: &RepoId::of(root),
            mmap_bytes: 0,
            now_ms: 1,
        })?)
    }

    /// Stores the file at `path`, with no symbol, and returns its node.
    fn add_file(writer: &mut Writer, path: &str) -> TestResult<NodeId> {
        let mut transaction = writer.transaction()?;
        transaction.replace_file(&FileIndex {
            file: FileRecord {
                path: RepoPath::new(path)?,
                language: None,
                size_bytes: 10,
                line_count: Some(1),
                content_hash: None,
                mtime_ns: 1,
                flags: FileFlags::empty(),
                indexed_at_ms: 1,
            },
            symbols: Vec::new(),
            refs: Vec::new(),
            passes_on: Vec::new(),
        })?;
        let node = transaction
            .queries()
            .cochange_files()?
            .into_iter()
            .find(|file| file.path.as_str() == path)
            .map(|file| file.node)
            .ok_or("the file's node")?;
        transaction.commit()?;
        Ok(node)
    }

    /// Sets or takes out the mark a complete mining leaves.
    fn mark(writer: &mut Writer, is_set: bool) -> TestResult {
        let mut transaction = writer.transaction()?;
        if is_set {
            transaction.set_git_state(GitStateKey::LastCommit, "abc")?;
        } else {
            transaction.remove_git_state(GitStateKey::LastCommit)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// A store with `a.txt` and `b.txt` and one co-change link, from `a.txt` to `b.txt`, and no
    /// mark yet.
    fn two_files(root: &Path) -> TestResult<Writer> {
        let mut writer = writer(root)?;
        let a = add_file(&mut writer, "a.txt")?;
        let b = add_file(&mut writer, "b.txt")?;
        let mut transaction = writer.transaction()?;
        transaction.set_edges(
            EdgeType::CoChange,
            &[(
                a,
                EdgeRecord {
                    dst: b,
                    weight: 1.0,
                    count: 2,
                    last_seq: 0,
                    is_directed: true,
                },
            )],
        )?;
        transaction.commit()?;
        Ok(writer)
    }

    /// What reads the graph of the store at `root`, as the daemon's does.
    fn reads_of(root: &Path) -> impl FnMut() -> Result<Read, String> + Send + 'static + use<> {
        store_reads(
            (&root.join(".cortex"), 0),
            GraphSettings::from(&Config::default()),
        )
    }

    /// Starts rebuilding a snapshot of the store at `root`.
    fn rebuilding(root: &Path) -> (Arc<Snapshot>, Rebuilder) {
        let snapshot = Arc::new(Snapshot::default());
        let rebuilder = Rebuilder::start(&snapshot, reads_of(root));
        (snapshot, rebuilder)
    }

    /// Waits until `snapshot` reports a graph `is_done` with, at most [`PATIENCE`].
    fn report_when(
        snapshot: &Snapshot,
        is_done: impl Fn(&GraphReport) -> bool,
    ) -> TestResult<GraphReport> {
        let until = Instant::now() + PATIENCE;
        loop {
            if let Some(report) = snapshot.report().filter(|report| is_done(report)) {
                return Ok(report);
            }
            if Instant::now() >= until {
                return Err(format!("still {:?}", snapshot.report()).into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits until the rebuilding thread has taken the change waiting, at most [`PATIENCE`].
    fn change_taken(snapshot: &Snapshot) -> TestResult {
        let until = Instant::now() + PATIENCE;
        while snapshot.wanted().is_changed {
            if Instant::now() >= until {
                return Err("the change was never taken".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    /// The graph is built once the thread starts, and again after a change: a third file, with no
    /// symbol, in the folder the others are in, adds its own node and nothing else.
    #[test]
    fn the_graph_is_built_and_rebuilt_after_a_change() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = two_files(root.path())?;
        let (snapshot, _rebuilder) = rebuilding(root.path());
        let first = report_when(&snapshot, |report| report.builds > 0)?;
        add_file(&mut writer, "c.txt")?;
        snapshot.changed();
        let second = report_when(&snapshot, |report| report.builds > first.builds)?;
        assert_eq!(second.nodes, first.nodes + 1);
        Ok(())
    }

    /// Co-change links are taken only from a complete mining: with no mark, the graph leaves out
    /// the one co-change link the store holds; with the mark, it holds it, one link more, since a
    /// co-change link runs one way.
    #[test]
    fn co_change_links_come_only_from_a_complete_mining() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = two_files(root.path())?;
        let (snapshot, _rebuilder) = rebuilding(root.path());
        let without = report_when(&snapshot, |report| report.builds > 0)?;
        assert!(!without.has_cochange);
        mark(&mut writer, true)?;
        snapshot.changed();
        let with = report_when(&snapshot, |report| report.builds > without.builds)?;
        assert!(with.has_cochange);
        assert_eq!(with.links, without.links + 1);
        Ok(())
    }

    /// While a mining is written, a rebuild that finds the mark taken out is let go and the graph
    /// built from the complete mining stands; the end of the write rebuilds it, here from a write
    /// cut short, which left no mark, and so without co-change links.
    #[test]
    fn the_graph_stands_while_a_mining_is_written() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = two_files(root.path())?;
        mark(&mut writer, true)?;
        let (snapshot, _rebuilder) = rebuilding(root.path());
        let complete = report_when(&snapshot, |report| report.has_cochange)?;
        let writing = snapshot.mining_write();
        mark(&mut writer, false)?;
        snapshot.changed();
        change_taken(&snapshot)?;
        // Once taken, the change is read at once: a rebuild period is time enough to let it go.
        std::thread::sleep(REBUILD_EVERY);
        assert_eq!(snapshot.report(), Some(complete.clone()));
        drop(writing);
        let after = report_when(&snapshot, |report| report.builds > complete.builds)?;
        assert!(!after.has_cochange);
        Ok(())
    }

    /// The first graph is taken even while a mining is written, without co-change links: there is
    /// no graph that could stand instead, and a store with no mark gives none of its links.
    #[test]
    fn the_first_graph_is_taken_while_a_mining_is_written() -> TestResult {
        let root = tempfile::tempdir()?;
        let _writer = two_files(root.path())?;
        let snapshot = Arc::new(Snapshot::default());
        let _writing = snapshot.mining_write();
        let _rebuilder = Rebuilder::start(&snapshot, reads_of(root.path()));
        let first = report_when(&snapshot, |report| report.builds > 0)?;
        assert!(!first.has_cochange);
        Ok(())
    }

    /// A mining's write is under way while what `mining_write` gave is held; its end asks for a
    /// rebuild.
    #[test]
    fn a_mining_write_is_told_while_it_is_held() {
        let snapshot = Snapshot::default();
        let writing = snapshot.mining_write();
        assert!(snapshot.is_mining());
        assert!(!snapshot.take_change());
        drop(writing);
        assert!(!snapshot.is_mining());
        assert!(snapshot.take_change());
    }

    /// A rebuild that fails is counted and tried again, here twice, a second and then two after
    /// the one before; the third read builds the graph, and the status keeps both failures.
    #[test]
    fn a_failed_rebuild_is_counted_and_tried_again() -> TestResult {
        let root = tempfile::tempdir()?;
        let _writer = two_files(root.path())?;
        let snapshot = Arc::new(Snapshot::default());
        let mut reads = reads_of(root.path());
        let mut tries = 0;
        let _rebuilder = Rebuilder::start(&snapshot, move || {
            tries += 1;
            if tries <= 2 {
                return Err("the disk is busy".to_owned());
            }
            reads()
        });
        let failed = report_when(&snapshot, |report| report.failed_builds == 1)?;
        assert_eq!(failed.builds, 0);
        let built = report_when(&snapshot, |report| report.builds == 1)?;
        assert_eq!(built.failed_builds, 2);
        assert!(built.nodes > 0);
        Ok(())
    }

    /// A rebuild that panics is counted as a failure, and the thread goes on to the next.
    #[test]
    #[allow(clippy::panic, reason = "the read under test panics on purpose")]
    fn a_panicking_rebuild_is_counted_and_tried_again() -> TestResult {
        let root = tempfile::tempdir()?;
        let _writer = two_files(root.path())?;
        let snapshot = Arc::new(Snapshot::default());
        let mut reads = reads_of(root.path());
        let mut has_panicked = false;
        let _rebuilder = Rebuilder::start(&snapshot, move || {
            if !std::mem::replace(&mut has_panicked, true) {
                panic!("a read that panics");
            }
            reads()
        });
        let built = report_when(&snapshot, |report| report.builds == 1)?;
        assert_eq!(built.failed_builds, 1);
        Ok(())
    }

    /// A stop is taken at once while the thread waits for the next rebuild: a change right after a
    /// build waits half a second from that build's start; a tenth of a second into that wait, the
    /// stop does not wait for the rest.
    #[test]
    fn a_stop_does_not_wait_for_the_next_rebuild() -> TestResult {
        let root = tempfile::tempdir()?;
        let _writer = two_files(root.path())?;
        let (snapshot, rebuilder) = rebuilding(root.path());
        report_when(&snapshot, |report| report.builds > 0)?;
        snapshot.changed();
        std::thread::sleep(REBUILD_EVERY / 5);
        let stopping = Instant::now();
        drop(rebuilder);
        let waited = stopping.elapsed();
        assert!(waited < REBUILD_EVERY / 2, "{waited:?}");
        Ok(())
    }

    /// A rebuild starts half a second after the one before, or four times as long as that one
    /// took when that is longer: 100 ms of rebuilding waits 0.5 s, 200 ms 0.8 s, and 2 s 8 s.
    /// After failures in a row it waits a second for one, two for two, four for three, and at
    /// most a minute: 64 s for seven is held to 60.
    #[test]
    fn rebuilds_are_paced_by_how_long_they_take_and_how_many_failed() {
        let started = Instant::now();
        let cases = [
            ((100, 0), 500),
            ((125, 0), 500),
            ((200, 0), 800),
            ((2_000, 0), 8_000),
            ((10, 1), 1_000),
            ((10, 2), 2_000),
            ((10, 3), 4_000),
            ((10, 7), 60_000),
            ((10, 40), 60_000),
            ((1_000, 1), 4_000),
        ];
        for ((took_ms, failures), wait_ms) in cases {
            let last = Last {
                started,
                took: Duration::from_millis(took_ms),
                failures,
            };
            assert_eq!(
                next_start(last),
                started + Duration::from_millis(wait_ms),
                "{took_ms} ms, {failures} failures"
            );
        }
    }

    /// Changes that come closer together than a rebuild period share rebuilds: fifty, one every
    /// 20 ms, cause one rebuild for each half second they span and at most two more (one at the
    /// first change, one for those that came while the last ran), where a rebuild for each would
    /// make about fifty.
    #[test]
    fn changes_close_together_share_a_rebuild() -> TestResult {
        let root = tempfile::tempdir()?;
        let _writer = two_files(root.path())?;
        let (snapshot, _rebuilder) = rebuilding(root.path());
        let first = report_when(&snapshot, |report| report.builds > 0)?;
        let started = Instant::now();
        for _ in 0..50 {
            snapshot.changed();
            std::thread::sleep(Duration::from_millis(20));
        }
        let periods = started.elapsed().as_millis() / REBUILD_EVERY.as_millis();
        report_when(&snapshot, |report| report.builds > first.builds)?;
        std::thread::sleep(REBUILD_EVERY * 2);
        let last = snapshot.report().ok_or("a graph")?;
        let builds = u128::from(last.builds - first.builds);
        assert!(
            builds <= periods + 2,
            "{builds} builds in {periods} periods"
        );
        Ok(())
    }
}
