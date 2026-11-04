//! The files the work tree holds changed against HEAD, read with `git status` on a thread of its
//! own (`paced`) after each write that may follow a change of them (an index, a stored mining) and
//! each move of HEAD the miner sees, turned into their file nodes and kept for retrievals, which
//! take them as they stand and never wait for git.
//!
//! A file Cortex does not read as code (generated, vendored, binary, too large, or named like a
//! secret) is left out, and of more than [`MAX_UNCOMMITTED_FILES`] the files the index saw change
//! last are kept; a diff of more than [`MAX_LISTED_FILES`] files is a mass change, of which none is
//! kept. A look that fails is counted, kept among the daemon's latest errors and tried again, and
//! until a look ends no file is kept, since what an earlier one read may no longer hold; nor is
//! any once the miner sees HEAD move, since a commit makes files uncommitted no longer, until a
//! look begun after the move ends. With no git program, no git repository at the root, a
//! repository whose own git configuration names a filter, or a partial clone with a git that would
//! fetch in it, there is nothing to look at, and the diff is unknown, as the status says.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cortex_core::ipc::{WorktreeReport, WorktreeState};
use cortex_core::{FileFlags, RepoPath};
use cortex_git::worktree::{WorktreeError, uncommitted_files};
use cortex_store::{NodeId, Queries, Reader, StoreError};

use crate::host::now_ms;
use crate::inbox::Inbox;
use crate::paced::{Asks, Paced, Pacing};

/// How the looks are spaced: at least two seconds apart, since each runs git; at least four times
/// as long after one as it took; after failures in a row four seconds after the first, twice as
/// long after each further one, at most ten minutes, so that a look that cannot succeed runs git
/// a few times an hour.
const PACING: Pacing = Pacing {
    every: Duration::from_secs(2),
    spacing: 4,
    max_retry_wait: Duration::from_secs(600),
};

/// The most files of the diff kept: 10, twice the five a work order lists, the files the index saw
/// change last, which are the work in progress. Seeded at 0.8 each they weigh at most 8 in all;
/// each older file of a larger diff would add weight to code the prompt may not be about.
pub(crate) const MAX_UNCOMMITTED_FILES: usize = 10;

/// The most files of a diff looked up in the store, a query each: a diff of more is a mass change
/// (a rename across the tree, a formatter's run, a generated tree left untracked), which says
/// little of one prompt.
const MAX_LISTED_FILES: usize = 5_000;

/// The flags of a file Cortex does not read as code, which seeds nothing however it changed.
const NOT_SEEDED: [FileFlags; 5] = [
    FileFlags::GENERATED,
    FileFlags::VENDORED,
    FileFlags::BINARY,
    FileFlags::TOO_LARGE,
    FileFlags::SECRET_PATTERN,
];

/// The uncommitted diff as last read, how the looks went, and the looks asked for.
#[derive(Debug, Default)]
pub(crate) struct Worktree {
    /// The diff and the looks.
    held: Mutex<Held>,
    /// The looks asked for.
    asks: Arc<Asks>,
}

/// The diff as last read, and how the looks since the daemon started went.
#[derive(Debug, Default)]
struct Held {
    /// The file nodes of the diff, as of the latest look: none before one ended, after one failed,
    /// or when the diff is not known.
    nodes: Option<Arc<Vec<NodeId>>>,
    /// Whether the latest look read the diff, or why it is not known; none before a look ended or
    /// failed.
    state: Option<WorktreeState>,
    /// The files git listed at the latest look.
    listed_files: u64,
    /// Looks that ended, failed ones not among them.
    looks: u64,
    /// Looks that failed; each was tried again.
    failed: u64,
    /// How many moves of HEAD the miner reported: a look begun before the latest is not taken in.
    head_moves: u64,
}

/// What one look found.
#[derive(Debug, PartialEq, Eq)]
struct Look {
    /// Whether it read the diff, or why the diff is not known.
    state: WorktreeState,
    /// The files git listed, whether they seed or not.
    listed_files: u64,
    /// The file nodes kept, when the diff was read.
    nodes: Option<Vec<NodeId>>,
}

impl Look {
    /// A look that found the diff not known, for the reason `state` gives.
    fn unknown(state: WorktreeState) -> Self {
        Self {
            state,
            listed_files: 0,
            nodes: None,
        }
    }
}

impl Worktree {
    /// The file nodes of the uncommitted diff as last read: none before the first look ended,
    /// after a look failed, or when the diff is not known.
    pub(crate) fn uncommitted(&self) -> Option<Arc<Vec<NodeId>>> {
        self.held().nodes.clone()
    }

    /// What asks for a look, for whatever learns of changes: the graph's snapshot asks it with
    /// every write it is told of (the miner asks through [`Worktree::head_moved`]).
    pub(crate) fn asks(&self) -> Arc<Asks> {
        Arc::clone(&self.asks)
    }

    /// What the daemon's status says of the work tree: none before a look ended or failed.
    pub(crate) fn report(&self) -> Option<WorktreeReport> {
        let held = self.held();
        Some(WorktreeReport {
            state: held.state?,
            uncommitted_files: held
                .nodes
                .as_ref()
                .map_or(0, |nodes| u64::try_from(nodes.len()).unwrap_or(u64::MAX)),
            listed_files: held.listed_files,
            looks: held.looks,
            failed_looks: held.failed,
        })
    }

    /// The diff and the looks, locked.
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lets go of the diff, which a commit or a checkout may have changed without a file the
    /// watcher reports, and asks for a look: what the miner does whenever it sees HEAD move. Until
    /// a look begun after this ends, retrievals take no file of the diff, and a diff that was read
    /// is said to wait for that look.
    pub(crate) fn head_moved(&self) {
        let mut held = self.held();
        held.head_moves += 1;
        if held.nodes.take().is_some() {
            held.state = Some(WorktreeState::HeadMoved);
            held.listed_files = 0;
        }
        drop(held);
        self.asks.ask();
    }

    /// How many moves of HEAD were reported, for a look to say when it began.
    fn head_moves(&self) -> u64 {
        self.held().head_moves
    }

    /// Takes in what a look found, which began when `head_moves` moves of HEAD were reported; a
    /// look begun before a later move is let go, the look that move asked for to follow.
    fn take(&self, look: Look, head_moves: u64) {
        let mut held = self.held();
        if held.head_moves != head_moves {
            return;
        }
        held.looks += 1;
        held.state = Some(look.state);
        held.listed_files = look.listed_files;
        held.nodes = look.nodes.map(Arc::new);
    }

    /// Counts a look that failed, after which no file is kept; the thread asks for another.
    fn fail(&self) {
        let mut held = self.held();
        held.failed += 1;
        held.state = Some(WorktreeState::Failed);
        held.listed_files = 0;
        held.nodes = None;
    }
}

/// Looks at the work tree of the repository at `root` at once, then after each change, as
/// [`PACING`] spaces the looks, turning what git lists into the file nodes the store in `folder`
/// holds, opened with `mmap_bytes` of memory map; a look that fails is counted and kept among
/// `inbox`'s latest errors. Stopped, and waited for, when what this returns is dropped.
pub(crate) fn watch(
    worktree: &Arc<Worktree>,
    root: &Path,
    (folder, mmap_bytes): (&Path, u64),
    inbox: &Arc<Inbox>,
) -> Paced {
    let (root, folder): (PathBuf, PathBuf) = (root.to_path_buf(), folder.to_path_buf());
    let (taking, failing) = (Arc::clone(worktree), Arc::clone(worktree));
    let noting = Arc::clone(inbox);
    let mut reader: Option<Reader> = None;
    Paced::start(
        "cortex-worktree",
        &worktree.asks,
        PACING,
        (
            move || {
                let began_at = taking.head_moves();
                let found = kept_of(uncommitted_files(&root), (&folder, mmap_bytes), &mut reader);
                taking.take(found?, began_at);
                Ok(())
            },
            move |why: &str| {
                failing.fail();
                tracing::warn!(error = %why, "the work tree's changes are not read");
                noting.note_error(&format!("the work tree's changes: {why}"), now_ms());
            },
        ),
    )
}

/// What a look keeps of `listed`, what git listed of the work tree or why it did not: the files,
/// as [`kept`] keeps them of `store`; the state that says why the diff is not known; or, for a
/// look that failed, why, in words.
fn kept_of(
    listed: Result<BTreeSet<RepoPath>, WorktreeError>,
    store: (&Path, u64),
    reader: &mut Option<Reader>,
) -> Result<Look, String> {
    let unknown = |state| Ok(Look::unknown(state));
    match listed {
        Ok(paths) => kept(&paths, store, reader),
        Err(WorktreeError::NoGit) => unknown(WorktreeState::NoGit),
        Err(WorktreeError::NoRepository) => unknown(WorktreeState::NoRepository),
        Err(WorktreeError::Refused) => unknown(WorktreeState::Refused),
        Err(WorktreeError::WouldFetch) => unknown(WorktreeState::WouldFetch),
        Err(WorktreeError::TooMany) => unknown(WorktreeState::TooMany),
        Err(error @ WorktreeError::Status(_)) => Err(error.to_string()),
    }
}

/// What a look keeps of `paths`, the files git listed: their file nodes as [`latest_nodes`] finds
/// them in the store in `folder`, through `reader`, opened with `mmap_bytes` of memory map when it
/// is not open yet and kept open; none, for a mass change, when they are more than
/// [`MAX_LISTED_FILES`].
fn kept(
    paths: &BTreeSet<RepoPath>,
    (folder, mmap_bytes): (&Path, u64),
    reader: &mut Option<Reader>,
) -> Result<Look, String> {
    let listed_files = u64::try_from(paths.len()).unwrap_or(u64::MAX);
    if paths.len() > MAX_LISTED_FILES {
        return Ok(Look {
            state: WorktreeState::TooMany,
            listed_files,
            nodes: None,
        });
    }
    let opened = match reader.take() {
        Some(opened) => opened,
        None => Reader::open(folder, mmap_bytes)
            .map_err(|error| format!("opening the store: {error:#}"))?,
    };
    let nodes = opened
        .consistently(|queries| latest_nodes(queries, paths))
        .map_err(|error| format!("{error:#}"));
    *reader = Some(opened);
    Ok(Look {
        state: WorktreeState::Read,
        listed_files,
        nodes: Some(nodes?),
    })
}

/// The file nodes `queries` holds for `paths`, at most [`MAX_UNCOMMITTED_FILES`] of them: those the
/// index saw change last, by the modification time it stored, the path breaking a tie. A path the
/// store holds no file node for, or whose file has one of the flags [`NOT_SEEDED`] names, is left
/// out.
fn latest_nodes(
    queries: Queries<'_>,
    paths: &BTreeSet<RepoPath>,
) -> Result<Vec<NodeId>, StoreError> {
    let mut held: Vec<(i64, &RepoPath, NodeId)> = Vec::new();
    for path in paths {
        let Some(row) = queries.file(path)? else {
            continue;
        };
        if NOT_SEEDED.iter().any(|flag| row.file.flags.contains(*flag)) {
            continue;
        }
        if let Some(node) = queries.file_node(row.id)? {
            held.push((row.file.mtime_ns, path, node));
        }
    }
    held.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(right.1)));
    Ok(held
        .into_iter()
        .take(MAX_UNCOMMITTED_FILES)
        .map(|(_, _, node)| node)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_store::{TestResult, add_file, add_flagged_file, writer};

    /// `names` as repository paths.
    fn paths(names: &[String]) -> TestResult<BTreeSet<RepoPath>> {
        Ok(names
            .iter()
            .map(|name| RepoPath::new(name))
            .collect::<Result<_, _>>()?)
    }

    /// Runs git with `args` in `folder`, reading no configuration but the repository's own.
    fn git(folder: &Path, args: &[&str]) -> TestResult {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(folder)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .status()?;
        if !status.success() {
            return Err(format!("git {args:?} failed: {status}").into());
        }
        Ok(())
    }

    /// A look that cannot read the diff says why as a state, not as a failure that would be tried
    /// again and again, the store never opened: each reason the reader gives, and, read from real
    /// folders, a folder without a repository and a repository whose own configuration names a
    /// filter, which no file even needs to use. Only git failing is a failure.
    #[test]
    fn a_look_says_why_it_reads_nothing() -> TestResult {
        let nowhere = tempfile::tempdir()?;
        let store = (nowhere.path(), 0);
        let cases = [
            (WorktreeError::NoGit, WorktreeState::NoGit),
            (WorktreeError::NoRepository, WorktreeState::NoRepository),
            (WorktreeError::Refused, WorktreeState::Refused),
            (WorktreeError::WouldFetch, WorktreeState::WouldFetch),
            (WorktreeError::TooMany, WorktreeState::TooMany),
        ];
        for (error, state) in cases {
            assert_eq!(kept_of(Err(error), store, &mut None)?, Look::unknown(state));
        }
        let failed = kept_of(
            Err(WorktreeError::Status("no".to_owned())),
            store,
            &mut None,
        );
        assert_eq!(
            failed,
            Err("listing the work tree's changes: no".to_owned())
        );
        assert_eq!(
            kept_of(uncommitted_files(nowhere.path()), store, &mut None)?,
            Look::unknown(WorktreeState::NoRepository)
        );
        let repo = tempfile::tempdir()?;
        git(repo.path(), &["init", "-q"])?;
        git(repo.path(), &["config", "filter.marking.clean", "cat"])?;
        assert_eq!(
            kept_of(uncommitted_files(repo.path()), store, &mut None)?,
            Look::unknown(WorktreeState::Refused)
        );
        Ok(())
    }

    /// When HEAD moves, the diff read before is let go at once and a look is asked for; a look
    /// that began before the move is not taken in, and one that began after it is. With no diff
    /// read, as in a repository refused, the state stays as it was.
    #[test]
    fn a_move_of_head_lets_the_diff_go() -> TestResult {
        let root = tempfile::tempdir()?;
        let node = add_file(&mut writer(root.path())?, "a.txt", 1)?;
        let read = || Look {
            state: WorktreeState::Read,
            listed_files: 2,
            nodes: Some(vec![node]),
        };
        let worktree = Worktree::default();
        worktree.take(read(), worktree.head_moves());
        let began_before = worktree.head_moves();
        assert!(!worktree.asks.take());
        worktree.head_moved();
        assert!(worktree.asks.take());
        assert_eq!(worktree.uncommitted(), None);
        let moved = worktree.report().ok_or("a report")?;
        assert_eq!(
            (
                moved.state,
                moved.uncommitted_files,
                moved.listed_files,
                moved.looks
            ),
            (WorktreeState::HeadMoved, 0, 0, 1)
        );
        worktree.take(read(), began_before);
        assert_eq!(worktree.uncommitted(), None);
        assert_eq!(worktree.report().ok_or("a report")?.looks, 1);
        worktree.take(read(), worktree.head_moves());
        assert_eq!(worktree.uncommitted().as_deref(), Some(&vec![node]));
        assert_eq!(
            worktree.report().ok_or("a report")?.state,
            WorktreeState::Read
        );
        worktree.take(Look::unknown(WorktreeState::Refused), worktree.head_moves());
        worktree.head_moved();
        assert_eq!(
            worktree.report().ok_or("a report")?.state,
            WorktreeState::Refused
        );
        Ok(())
    }

    /// Of the diff's files the store holds, those the index saw change last come first, the path
    /// breaking a tie, and a path the store does not hold is left out; past 10, the oldest go.
    /// Worked by hand: `b` changed at 30, `c` and `d` at 20 and `a` at 10, so b, c, d, a; of 11
    /// files changed at 1 to 11, the 10 changed at 11 down to 2.
    #[test]
    fn the_files_changed_last_are_kept() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let a = add_file(&mut writer, "a.txt", 10)?;
        let b = add_file(&mut writer, "b.txt", 30)?;
        let d = add_file(&mut writer, "d.txt", 20)?;
        let c = add_file(&mut writer, "c.txt", 20)?;
        let reader = Reader::open(&root.path().join(".cortex"), 0)?;
        let named: Vec<String> = ["a.txt", "b.txt", "c.txt", "d.txt", "gone.txt"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let few = paths(&named)?;
        assert_eq!(
            reader.consistently(|queries| latest_nodes(queries, &few))?,
            [b, c, d, a]
        );
        let names: Vec<String> = (1..=11).map(|at| format!("m{at:02}.txt")).collect();
        let mut nodes = Vec::new();
        for (at, name) in (1..).zip(&names) {
            nodes.push(add_file(&mut writer, name, at)?);
        }
        let many = paths(&names)?;
        let expected: Vec<NodeId> = nodes.iter().skip(1).rev().copied().collect();
        assert_eq!(expected.len(), 10);
        assert_eq!(
            reader.consistently(|queries| latest_nodes(queries, &many))?,
            expected
        );
        Ok(())
    }

    /// A file Cortex does not read as code seeds nothing, however lately it changed: of a file
    /// generated, one vendored, one binary, one too large and one named like a secret, all changed
    /// at 50, and a plain one changed at 10, only the plain one is kept.
    #[test]
    fn files_not_read_as_code_are_left_out() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        for (name, flag) in [
            ("gen.py", FileFlags::GENERATED),
            ("vendor/lib.py", FileFlags::VENDORED),
            ("logo.png", FileFlags::BINARY),
            ("dump.sql", FileFlags::TOO_LARGE),
            (".env", FileFlags::SECRET_PATTERN),
        ] {
            add_flagged_file(&mut writer, name, 50, flag)?;
        }
        let plain = add_file(&mut writer, "a.py", 10)?;
        let reader = Reader::open(&root.path().join(".cortex"), 0)?;
        let named: Vec<String> = [
            "gen.py",
            "vendor/lib.py",
            "logo.png",
            "dump.sql",
            ".env",
            "a.py",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let listed = paths(&named)?;
        assert_eq!(
            reader.consistently(|queries| latest_nodes(queries, &listed))?,
            [plain]
        );
        Ok(())
    }

    /// A diff of 5,000 files is looked up, one of 5,001 is a mass change of which none is kept,
    /// looked up or not, the store not even opened.
    #[test]
    fn a_mass_change_keeps_nothing() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let kept_one = add_file(&mut writer, "f0001.py", 1)?;
        let folder = root.path().join(".cortex");
        let names: Vec<String> = (1..=5_001).map(|at| format!("f{at:04}.py")).collect();
        let most = paths(names.get(..5_000).ok_or("5,000 names")?)?;
        let mut reader = None;
        assert_eq!(
            kept(&most, (&folder, 0), &mut reader)?,
            Look {
                state: WorktreeState::Read,
                listed_files: 5_000,
                nodes: Some(vec![kept_one]),
            }
        );
        let elsewhere = tempfile::tempdir()?;
        let too_many = paths(&names)?;
        assert_eq!(
            kept(&too_many, (elsewhere.path(), 0), &mut None)?,
            Look {
                state: WorktreeState::TooMany,
                listed_files: 5_001,
                nodes: None,
            }
        );
        Ok(())
    }

    /// The status says nothing before a look ended; after one, its state, how many files git
    /// listed and how many it kept, and how many looks there were; after a failure, that the
    /// latest look failed, with no files kept, the ones before let go; after a look that found no
    /// git program, that git is missing.
    #[test]
    fn the_status_says_how_the_looks_went() -> TestResult {
        let root = tempfile::tempdir()?;
        let node = add_file(&mut writer(root.path())?, "a.txt", 1)?;
        let worktree = Worktree::default();
        assert_eq!(worktree.report(), None);
        worktree.take(
            Look {
                state: WorktreeState::Read,
                listed_files: 3,
                nodes: Some(vec![node]),
            },
            0,
        );
        assert_eq!(
            worktree.report(),
            Some(WorktreeReport {
                state: WorktreeState::Read,
                uncommitted_files: 1,
                listed_files: 3,
                looks: 1,
                failed_looks: 0,
            })
        );
        assert_eq!(worktree.uncommitted().as_deref(), Some(&vec![node]));
        worktree.fail();
        assert_eq!(
            worktree.report(),
            Some(WorktreeReport {
                state: WorktreeState::Failed,
                uncommitted_files: 0,
                listed_files: 0,
                looks: 1,
                failed_looks: 1,
            })
        );
        assert_eq!(worktree.uncommitted(), None);
        worktree.take(Look::unknown(WorktreeState::NoGit), 0);
        assert_eq!(
            worktree.report(),
            Some(WorktreeReport {
                state: WorktreeState::NoGit,
                uncommitted_files: 0,
                listed_files: 0,
                looks: 2,
                failed_looks: 1,
            })
        );
        assert_eq!(worktree.uncommitted(), None);
        Ok(())
    }
}
