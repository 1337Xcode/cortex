//! Watching a repository for changes. Events from the file system are gathered until a path has
//! been quiet for 200 ms, then sorted by the walk's own rules: paths under `.git` or `.cortex`,
//! and paths the ignore files and patterns leave out, are dropped; created, changed, removed and
//! renamed paths (both names of a rename) are reported together. More than 500 paths within
//! 2 seconds, as a branch switch or a large checkout makes, or a change to a file of rules the walk
//! reads (`.gitignore`, `.cortexignore`, `.gitattributes`, `.git/info`; not one in a folder the
//! walk leaves out), is reported as a change to everything, to be looked at whole rather than path
//! by path. So is an error from the watcher, which may have lost events.
//!
//! Events are sorted on a thread of the watcher's own, which keeps the rules of each folder it
//! has looked at; they are loaded again when a file of rules changes.

use std::collections::{BTreeSet, VecDeque};
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cortex_core::RepoPath;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, NoCache, new_debouncer_opt};
use thiserror::Error;

use crate::discover::{DiscoverError, DiscoverOptions};
use crate::seen::{PathRules, WalkRules};

/// How long a path must be quiet before its events are reported.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// More changed paths than this within [`BATCH_WINDOW`] are reported as a change to everything.
pub(crate) const MAX_PATHS_IN_WINDOW: usize = 500;

/// The window [`MAX_PATHS_IN_WINDOW`] counts paths over.
const BATCH_WINDOW: Duration = Duration::from_secs(2);

/// How many batches of events may wait for the sorting thread; past this the file system
/// watcher's own thread waits too, so a slow consumer holds memory to this bound.
const WAITING_BATCHES: usize = 1024;

/// What changed in a watched repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// These paths, which the walk would find or found: created, changed, removed or renamed. A
    /// folder created, removed or renamed may be reported by its own path alone, which stands for
    /// everything under it.
    Paths(BTreeSet<RepoPath>),
    /// Too much, or the rules about which files count: everything is to be looked at again.
    Everything,
}

/// Why watching could not start.
#[derive(Debug, Error)]
pub enum WatchError {
    /// The repository's rules could not be read.
    #[error(transparent)]
    Rules(#[from] DiscoverError),
    /// The file system would not report changes.
    #[error("the repository cannot be watched: {0}")]
    Watch(#[from] notify::Error),
}

/// A repository being watched. Dropping it stops watching; no change is reported after that.
pub struct Watcher {
    /// The source of events, gathered and quieted.
    debouncer: Option<Debouncer<RecommendedWatcher, NoCache>>,
    /// The thread that sorts events into changes.
    sorter: Option<JoinHandle<()>>,
}

impl Drop for Watcher {
    /// Stops the events, then waits for the last of them to be sorted.
    fn drop(&mut self) {
        if let Some(debouncer) = self.debouncer.take() {
            debouncer.stop();
        }
        if let Some(sorter) = self.sorter.take() {
            // A sorter that panicked has nothing left to report.
            let _ = sorter.join();
        }
    }
}

/// Watches the repository at `root`, reporting each change to `report` from a thread of its own,
/// with the walk's `options` deciding which paths count.
///
/// # Errors
///
/// [`WatchError::Rules`] when the repository's rules cannot be read, and [`WatchError::Watch`]
/// when the file system will not report its changes.
pub fn watch(
    root: &Path,
    options: &DiscoverOptions<'_>,
    report: impl FnMut(Change) + Send + 'static,
) -> Result<Watcher, WatchError> {
    let walk = WalkRules::of(root, options);
    // The rules are read once here, so that a repository whose rules cannot be read is refused.
    walk.repository()?;
    let (sender, receiver) = mpsc::sync_channel(WAITING_BATCHES);
    // A send fails only once the sorting thread has stopped, when nothing is left to report to.
    let handler = move |events: DebounceEventResult| drop(sender.send(events));
    // Links are not followed, as the walk does not follow them: a change outside the repository
    // must not be reported as one inside it.
    let config = notify::Config::default().with_follow_symlinks(false);
    // No file id cache: the debouncer's own, on Windows and macOS, walks every folder it watches
    // and every folder created in it, following links and opening each file for its id, a walk
    // that would leave the repository and touch files that may hold secrets. It only pairs the two
    // names of a rename, and both are reported without it.
    let mut debouncer = new_debouncer_opt::<_, RecommendedWatcher, NoCache>(
        DEBOUNCE,
        None,
        handler,
        NoCache::new(),
        config,
    )?;
    debouncer.watch(root, RecursiveMode::Recursive)?;
    let sorter = thread::spawn(move || sort_events(&receiver, walk, report));
    Ok(Watcher {
        debouncer: Some(debouncer),
        sorter: Some(sorter),
    })
}

/// Sorts the events `receiver` brings into changes for `report`, until the events stop.
fn sort_events(
    receiver: &mpsc::Receiver<DebounceEventResult>,
    walk: WalkRules,
    mut report: impl FnMut(Change),
) {
    let Ok(mut filter) = PathRules::new(walk) else {
        return;
    };
    let mut recent: VecDeque<(Instant, usize)> = VecDeque::new();
    for events in receiver {
        let Ok(events) = events else {
            report(Change::Everything);
            continue;
        };
        let natives = events.iter().flat_map(|event| event.paths.iter());
        let paths: BTreeSet<RepoPath> = natives
            .filter_map(|native| filter.relative(native))
            .collect();
        if paths.iter().any(|path| filter.changes_rules(path)) {
            filter.reload();
            recent.clear();
            report(Change::Everything);
            continue;
        }
        let seen: BTreeSet<RepoPath> = paths
            .into_iter()
            .filter(|path| filter.is_seen(path))
            .collect();
        if seen.is_empty() {
            continue;
        }
        let now = Instant::now();
        while recent
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > BATCH_WINDOW)
        {
            recent.pop_front();
        }
        recent.push_back((now, seen.len()));
        if recent.iter().map(|(_, count)| count).sum::<usize>() > MAX_PATHS_IN_WINDOW {
            recent.clear();
            report(Change::Everything);
        } else {
            report(Change::Paths(seen));
        }
    }
}
