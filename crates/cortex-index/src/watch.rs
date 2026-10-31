//! Watching a repository for changes. Events from the file system are gathered until a path has
//! been quiet for 200 ms, then sorted by the walk's own rules: paths under `.git` or `.cortex`,
//! and paths the ignore files and patterns leave out, are dropped; created, changed, removed and
//! renamed paths (both names of a rename) are reported together. More than 500 paths within
//! 2 seconds, as a branch switch or a large checkout makes, or a change to a file of rules
//! (`.gitignore`, `.cortexignore`, `.gitattributes`, `.git/info`), is reported as a change to
//! everything, to be looked at whole rather than path by path. So is an error from the watcher,
//! which may have lost events.
//!
//! Events are sorted on a thread of the watcher's own, which keeps the rules of each folder it
//! has looked at; they are loaded again when a file of rules changes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cortex_core::RepoPath;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use thiserror::Error;

use crate::discover::{DiscoverError, DiscoverOptions, MAX_FOLDER_DEPTH, PRUNED_NAMES};
use crate::rules::{FolderRules, Reading, RepositoryRules};

/// How long a path must be quiet before its events are reported.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// More changed paths than this within [`BATCH_WINDOW`] are reported as a change to everything.
const MAX_PATHS_IN_WINDOW: usize = 500;

/// The window [`MAX_PATHS_IN_WINDOW`] counts paths over.
const BATCH_WINDOW: Duration = Duration::from_secs(2);

/// The names of the files that hold rules about which files the walk finds.
const RULES_FILES: [&str; 3] = [".gitignore", ".cortexignore", ".gitattributes"];

/// What changed in a watched repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// These paths, which the walk would find or found: created, changed, removed or renamed.
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
    debouncer: Option<Debouncer<RecommendedWatcher, RecommendedCache>>,
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
    let walk = WalkRules {
        root: root.to_owned(),
        extra_ignores: options.extra_ignores.to_vec(),
        global_gitignore: options.global_gitignore.map(Path::to_owned),
        max_file_bytes: options.max_file_bytes,
    };
    // The rules are read once here, so that a repository whose rules cannot be read is refused.
    walk.repository()?;
    let (sender, receiver) = mpsc::channel();
    let mut debouncer = new_debouncer(DEBOUNCE, None, sender)?;
    debouncer.watch(root, RecursiveMode::Recursive)?;
    let sorter = thread::spawn(move || sort_events(&receiver, walk, report));
    Ok(Watcher {
        debouncer: Some(debouncer),
        sorter: Some(sorter),
    })
}

/// What the walk's rules are read from.
struct WalkRules {
    /// The repository's root.
    root: PathBuf,
    /// `index.extra_ignores`.
    extra_ignores: Vec<String>,
    /// The user's global gitignore, if any.
    global_gitignore: Option<PathBuf>,
    /// The largest rules file read.
    max_file_bytes: u64,
}

impl WalkRules {
    /// The repository-wide rules, read now.
    fn repository(&self) -> Result<RepositoryRules, DiscoverError> {
        let mut problems = Vec::new();
        let mut reading = Reading {
            max_bytes: self.max_file_bytes,
            problems: &mut problems,
        };
        RepositoryRules::load(
            &self.root,
            &self.extra_ignores,
            self.global_gitignore.as_deref(),
            &mut reading,
        )
    }
}

/// Sorts the events `receiver` brings into changes for `report`, until the events stop.
fn sort_events(
    receiver: &mpsc::Receiver<DebounceEventResult>,
    walk: WalkRules,
    mut report: impl FnMut(Change),
) {
    let Ok(mut filter) = Filter::new(walk) else {
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
        if paths.iter().any(is_rules_file) {
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

/// Whether `path` holds rules about which files the walk finds.
fn is_rules_file(path: &RepoPath) -> bool {
    RULES_FILES.contains(&path.file_name()) || path.as_str().starts_with(".git/info/")
}

/// The walk's rules as events are sorted by them, each folder's read once.
struct Filter {
    /// Where the rules are read from.
    walk: WalkRules,
    /// The repository's root as written and, where it differs, as the file system spells it.
    roots: Vec<PathBuf>,
    /// The repository-wide rules.
    repository: RepositoryRules,
    /// The rules of each folder looked at, by path.
    folders: BTreeMap<RepoPath, Rc<FolderRules>>,
}

impl Filter {
    /// The rules of the repository `walk` describes.
    fn new(walk: WalkRules) -> Result<Self, DiscoverError> {
        let repository = walk.repository()?;
        let mut roots = vec![walk.root.clone()];
        if let Ok(canonical) = walk.root.canonicalize() {
            roots.push(canonical);
        }
        Ok(Self {
            walk,
            roots,
            repository,
            folders: BTreeMap::new(),
        })
    }

    /// Reads the rules again, after a file of rules changed.
    fn reload(&mut self) {
        if let Ok(repository) = self.walk.repository() {
            self.repository = repository;
        }
        self.folders.clear();
    }

    /// The repository path of the file system path `native`, if it is inside the repository.
    fn relative(&self, native: &Path) -> Option<RepoPath> {
        let inside = self
            .roots
            .iter()
            .find_map(|root| native.strip_prefix(root).ok())?;
        let parts: Vec<&str> = inside
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect::<Option<_>>()?;
        RepoPath::new(&parts.join("/")).ok()
    }

    /// Whether the walk would look at the path `path`: no folder above it is pruned, ignored or
    /// too deep, and the path itself is not ignored.
    fn is_seen(&mut self, path: &RepoPath) -> bool {
        let parts: Vec<&str> = path.as_str().split('/').collect();
        if parts.len() > MAX_FOLDER_DEPTH + 1 {
            return false;
        }
        let mut folder = self.rules_of(&RepoPath::root(), None);
        let mut at = RepoPath::root();
        for (position, part) in parts.iter().enumerate() {
            if PRUNED_NAMES.contains(part) {
                return false;
            }
            let Ok(next) = at.join(part) else {
                return false;
            };
            let native = next.to_native(&self.walk.root);
            let is_last = position + 1 == parts.len();
            let is_dir = !is_last || native.is_dir();
            if self.repository.is_ignored(&folder, &native, is_dir) {
                return false;
            }
            if !is_last {
                folder = self.rules_of(&next, Some(folder));
            }
            at = next;
        }
        true
    }

    /// The rules of the folder at `path`, inside the folder `parent` describes, read once.
    fn rules_of(&mut self, path: &RepoPath, parent: Option<Rc<FolderRules>>) -> Rc<FolderRules> {
        if let Some(rules) = self.folders.get(path) {
            return Rc::clone(rules);
        }
        let native = path.to_native(&self.walk.root);
        let mut problems = Vec::new();
        let mut reading = Reading {
            max_bytes: self.walk.max_file_bytes,
            problems: &mut problems,
        };
        let is_listed = |name: &str| native.join(name).is_file();
        let rules = Rc::new(FolderRules::load(
            &native,
            path.clone(),
            parent,
            &is_listed,
            &mut reading,
        ));
        self.folders.insert(path.clone(), Rc::clone(&rules));
        rules
    }
}
