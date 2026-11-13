//! The graph retrieval spreads over, held in memory and rebuilt whole from the store after a write
//! changes what it holds: an index, a stored mining, a task's learning.
//!
//! A request takes the graph as it stands and never waits for a rebuild. The rebuilds run on a
//! thread of their own (`paced`), each built from one read transaction, so that it sees the store
//! as one moment left it, and swapped in whole. However often the store changes, a rebuild starts
//! at least half a second after the one before, and at least four times as long after it as that
//! one took, so that rebuilding a large graph takes at most a quarter of one core. A rebuild that
//! fails, or panics, is counted, kept out of the graph, and tried again, a second later after one
//! failure, twice as long after each further one in a row, at most a minute; the status says how
//! many failed. A stop is taken at once, between rebuilds or while waiting for the next.
//!
//! A huge repository's graph is also viewed by files as it is rebuilt, since its retrievals spread
//! over files first: that view is built here, off the requests' way, and kept with the graph.
//!
//! Co-change links are taken only from a complete mining. The miner takes its mark out of the store
//! before it writes and puts it back once done, and the host holds a [`MiningWrite`] while it
//! writes. A rebuild that finds no mark while a write is under way is let go, the graph standing,
//! since the end of the write asks for another; one that finds no mark otherwise (no mining yet, or
//! one cut short), or that would be the first graph, is built without co-change links.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cortex_core::ScaleMode;
use cortex_core::ipc::GraphReport;
use cortex_retrieve::{Graph, GraphSettings, RetrieveError, read_graph};
use cortex_store::{GitStateKey, NodeId, Reader};

use crate::paced::{Asks, Paced, Pacing};
use crate::retrieval::mode_of;

/// How the rebuilds are spaced: at least half a second apart, so that writes that come closer
/// together share one; at least four times as long after one as it took, so that rebuilding takes
/// at most a quarter of the thread's time; and at most a minute after failures.
const PACING: Pacing = Pacing {
    every: Duration::from_millis(500),
    spacing: 4,
    max_retry_wait: Duration::from_secs(60),
};

/// A graph, with whether it holds the co-change links of a complete mining.
pub(crate) type Read = (Graph<NodeId>, bool);

/// The graph as it stands, and what the thread that rebuilds it waits for.
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    /// The graph, once one is built, and how the rebuilds went.
    held: Mutex<Held>,
    /// The rebuilds asked for.
    asks: Arc<Asks>,
    /// Whether a mining is being written.
    is_mining: AtomicBool,
    /// How many times the store was noted as changed: what holds a copy of the store's files,
    /// as the map's listing does, compares it with the number it was built at.
    generation: AtomicU64,
    /// The work that follows the store's changes as the graph does, asked whenever it is.
    followers: Mutex<Vec<Arc<Asks>>>,
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
    /// The snapshot's generation when the read began: a change noted since makes it stale.
    generation: u64,
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
        self.snapshot.is_mining.store(false, Ordering::SeqCst);
        self.snapshot.changed();
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
            is_stale: built.is_none_or(|built| built.generation != self.generation()),
        })
    }

    /// Notes that the store changed, so that the graph is rebuilt and its followers asked.
    pub(crate) fn changed(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.asks.ask();
        let followers = self
            .followers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for follower in followers.iter() {
            follower.ask();
        }
    }

    /// How many times the store was noted as changed since the daemon started; a copy of the
    /// store built at one number is out of date at any other.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Has `asks` asked whenever the graph is: work that a change of the store may concern too.
    pub(crate) fn follow(&self, asks: Arc<Asks>) {
        self.followers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(asks);
    }

    /// Notes that a mining's write begins, until what this returns is dropped.
    pub(crate) fn mining_write(&self) -> MiningWrite<'_> {
        self.is_mining.store(true, Ordering::SeqCst);
        MiningWrite { snapshot: self }
    }

    /// The graph and the rebuilds' counts, locked.
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the change waiting, as the rebuilding thread does: whether the store changed since
    /// the last time. For the tests of what tells the snapshot.
    #[cfg(test)]
    pub(crate) fn take_change(&self) -> bool {
        self.asks.take()
    }

    /// Whether a mining's write is under way, as the snapshot was told. For tests.
    #[cfg(test)]
    pub(crate) fn is_mining(&self) -> bool {
        self.is_mining.load(Ordering::SeqCst)
    }

    /// Takes in `read`, a graph read from the store in a read begun at `generation`, unless a
    /// mining is being written and the graph standing came from before it: a read that finds the
    /// mark out then holds part of the write.
    fn take(&self, (graph, has_cochange): Read, (took, generation): (Duration, u64)) {
        let is_mining = self.is_mining.load(Ordering::SeqCst);
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
                generation,
            });
        }
        tracing::info!(nodes, links, has_cochange, took_ms, "graph rebuilt");
    }

    /// Counts a rebuild that failed; the thread asks for another.
    fn fail(&self, why: &str) {
        self.held().failed += 1;
        tracing::warn!(error = %why, "the graph is not rebuilt");
    }
}

/// Rebuilds `snapshot` with what `read` reads: at once, then after every change, as [`PACING`]
/// spaces the rebuilds; a read that fails or panics is counted and tried again. A thread that
/// cannot start is logged, and the daemon retrieves nothing. Stopped, and waited for, when what
/// this returns is dropped.
pub(crate) fn rebuild(
    snapshot: &Arc<Snapshot>,
    mut read: impl FnMut() -> Result<Read, String> + Send + 'static,
) -> Paced {
    let (taking, failing) = (Arc::clone(snapshot), Arc::clone(snapshot));
    Paced::start(
        "cortex-graph",
        &snapshot.asks,
        PACING,
        (
            move || {
                let started = Instant::now();
                // Read before the store is: a change noted while it reads makes the graph stale.
                let generation = taking.generation();
                let graph = read()?;
                taking.take(graph, (started.elapsed(), generation));
                Ok(())
            },
            move |why: &str| failing.fail(why),
        ),
    )
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
/// with its co-change links only when the miner's mark is there, and viewed by files, once the
/// transaction is over, when the store is in huge mode.
fn read_whole(reader: &Reader, settings: &GraphSettings) -> Result<Read, RetrieveError> {
    let (graph, has_cochange, is_huge) =
        reader.consistently(|queries| -> Result<_, RetrieveError> {
            let has_cochange = queries.git_state(GitStateKey::LastCommit)?.is_some();
            let mut weighed = *settings;
            if !has_cochange {
                weighed.type_weights.cochange = 0.0;
            }
            let is_huge = mode_of(queries)? == Some(ScaleMode::Huge);
            Ok((read_graph(queries, &weighed)?, has_cochange, is_huge))
        })?;
    if is_huge {
        graph.prepare_by_files()?;
    }
    Ok((graph, has_cochange))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::AtomicU32;
    use std::time::{Duration, Instant};

    use cortex_core::ipc::GraphReport;
    use cortex_core::{Config, EdgeType};
    use cortex_store::{EdgeRecord, GitStateKey, Writer};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// How long a test waits for a rebuild it expects.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// How long the first build takes in the stop's case. The next may start only four times as
    /// long after its start, eight seconds, so a stop that waited for it would wait about six:
    /// three times [`STOP_TAKEN_WITHIN`], which a loaded machine's own delays stay far below.
    const SLOW_BUILD: Duration = Duration::from_secs(2);

    /// How soon a stop must be taken for it not to have waited for the next rebuild. Shorter
    /// bounds measured the machine: a shared CI runner took 816 ms to wake and join the thread.
    const STOP_TAKEN_WITHIN: Duration = Duration::from_secs(2);

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
        let a = add_file(&mut writer, "a.txt", 1)?;
        let b = add_file(&mut writer, "b.txt", 1)?;
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
    fn rebuilding(root: &Path) -> (Arc<Snapshot>, Paced) {
        let snapshot = Arc::new(Snapshot::default());
        let rebuilder = rebuild(&snapshot, reads_of(root));
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
        while snapshot.asks.is_asked() {
            if Instant::now() >= until {
                return Err("the change was never taken".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    /// A graph is stale once a change is noted after its read began, and current again once a read
    /// begun after the change is taken: taken at the generation its read began at, a graph read
    /// before a change stays stale, as when the change is noted while it reads. Taken by hand, with
    /// no rebuilding thread, so that the order is the test's.
    #[test]
    fn a_graph_read_before_a_change_is_stale() -> TestResult {
        let root = tempfile::tempdir()?;
        let _writer = two_files(root.path())?;
        let mut read = reads_of(root.path());
        let snapshot = Snapshot::default();
        let begun = snapshot.generation();
        snapshot.take(read()?, (Duration::ZERO, begun));
        assert_eq!(snapshot.report().map(|report| report.is_stale), Some(false));
        snapshot.changed();
        assert_eq!(snapshot.report().map(|report| report.is_stale), Some(true));
        snapshot.take(read()?, (Duration::ZERO, begun));
        assert_eq!(snapshot.report().map(|report| report.is_stale), Some(true));
        snapshot.take(read()?, (Duration::ZERO, snapshot.generation()));
        assert_eq!(snapshot.report().map(|report| report.is_stale), Some(false));
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
        add_file(&mut writer, "c.txt", 1)?;
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
        std::thread::sleep(PACING.every);
        // The graph stands, and says the store has changed since it was read.
        let standing = GraphReport {
            is_stale: true,
            ..complete.clone()
        };
        assert_eq!(snapshot.report(), Some(standing));
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
        let _rebuilder = rebuild(&snapshot, reads_of(root.path()));
        let first = report_when(&snapshot, |report| report.builds > 0)?;
        assert!(!first.has_cochange);
        Ok(())
    }

    /// A huge repository's graph comes viewed by files, so that no prompt waits for the view; any
    /// other's does not, and the view is left until something spreads in two stages over it. The
    /// store starts in no mode, is rebuilt, then is put in huge mode and rebuilt again.
    #[test]
    fn a_huge_repositorys_graph_comes_viewed_by_files() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = two_files(root.path())?;
        let (snapshot, _rebuilder) = rebuilding(root.path());
        let first = report_when(&snapshot, |report| report.builds > 0)?;
        let graph = snapshot.graph().ok_or("no graph after the first build")?;
        assert!(!graph.is_prepared_by_files());
        let mut transaction = writer.transaction()?;
        transaction.set_scale_mode(ScaleMode::Huge)?;
        transaction.commit()?;
        snapshot.changed();
        report_when(&snapshot, |report| report.builds > first.builds)?;
        let graph = snapshot.graph().ok_or("no graph after the second build")?;
        assert!(graph.is_prepared_by_files());
        Ok(())
    }

    /// A follower is asked whenever the graph is: at a change, and at the end of a mining's write.
    #[test]
    fn a_follower_is_asked_with_the_graph() {
        let snapshot = Snapshot::default();
        let follower = Arc::new(Asks::default());
        snapshot.follow(Arc::clone(&follower));
        snapshot.changed();
        assert!(follower.take());
        let writing = snapshot.mining_write();
        assert!(!follower.take());
        drop(writing);
        assert!(follower.take());
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
        let _rebuilder = rebuild(&snapshot, move || {
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
        let _rebuilder = rebuild(&snapshot, move || {
            if !std::mem::replace(&mut has_panicked, true) {
                panic!("a read that panics");
            }
            reads()
        });
        let built = report_when(&snapshot, |report| report.builds == 1)?;
        assert_eq!(built.failed_builds, 1);
        Ok(())
    }

    /// A stop is taken at once while the thread waits for the next rebuild: a first build that
    /// took two seconds spaces the next eight seconds from its start; a tenth of a second after a
    /// change, the stop does not wait the six seconds left, and no second build starts.
    #[test]
    fn a_stop_does_not_wait_for_the_next_rebuild() -> TestResult {
        let root = tempfile::tempdir()?;
        let _writer = two_files(root.path())?;
        let reads = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&reads);
        let mut store = reads_of(root.path());
        let snapshot = Arc::new(Snapshot::default());
        let rebuilder = rebuild(&snapshot, move || {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                std::thread::sleep(SLOW_BUILD);
            }
            store()
        });
        report_when(&snapshot, |report| report.builds > 0)?;
        snapshot.changed();
        std::thread::sleep(PACING.every / 5);
        let stopping = Instant::now();
        drop(rebuilder);
        let waited = stopping.elapsed();
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(waited < STOP_TAKEN_WITHIN, "{waited:?}");
        Ok(())
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
        let periods = started.elapsed().as_millis() / PACING.every.as_millis();
        report_when(&snapshot, |report| report.builds > first.builds)?;
        std::thread::sleep(PACING.every * 2);
        let last = snapshot.report().ok_or("a graph")?;
        let builds = u128::from(last.builds - first.builds);
        assert!(
            builds <= periods + 2,
            "{builds} builds in {periods} periods"
        );
        Ok(())
    }
}
