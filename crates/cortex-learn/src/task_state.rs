//! What each open task did, kept as its events are recorded, for what is learned when it ends.
//!
//! A task's state follows its event rows: each read, edit, search and shell command the store
//! keeps for the task is a step, in the order recorded. A read, edit or search whose tool reported
//! a failure is left out. Three facts the rows do not keep are added while the daemon runs: the
//! files a search found, the git restores of a shell line that exited 0, and what each file the
//! task edited held before the task first changed it. A state not in memory, because the daemon
//! started again or dropped it for room, is rebuilt from the task's rows when the task is next
//! touched, without those three. When the task ends its state is handed out, each of its files
//! given the graph node the store has for the file's path by then.
//!
//! What the states hold is bounded however long the daemon runs: so many states, so many rows a
//! state is built from (and a file's earlier content only with a row), and so many bytes of found
//! files a state keeps.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{
    CheckCounts, CommandKind, EventKind, FileContent, Identifier, LineRange, RepoPath, Ulid,
};
use cortex_store::{EventRecord, NodeId, Queries, StoreError};

use crate::commands::Restore;

/// Most event rows of one task its state is built from. A task with more keeps the steps of its
/// first rows and is marked partial, so that one endless task cannot take the daemon's memory.
pub const MAX_TASK_ROWS: usize = 4_096;

/// Most task states held at once. Past that, the state touched longest ago is dropped, to be
/// rebuilt from its rows if its task is touched again: tasks whose session died without ending
/// them stay open, and must not pile up.
pub const MAX_TASK_STATES: usize = 32;

/// Most bytes the paths of the files a task's searches found hold together in its state: sixteen
/// searches at the most a search's hits hold. The files found first are kept until one would pass
/// it, and none after: a file a search found is the weakest thing a task did.
pub const MAX_TASK_HIT_BYTES: usize = 64 * 1024;

/// How a task ended, which tells whether the agent got to the end of an answer about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEnd {
    /// The answer to it ended.
    Answered,
    /// It was left: the session's next prompt came, or the session ended, with no end of an
    /// answer, as when the agent was interrupted or stopped.
    Left,
    /// Its turn stopped on an error, such as a usage limit, before its answer ended.
    Stopped,
}

/// A file a step touched, and its graph node when the store has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNode {
    /// The file.
    pub path: RepoPath,
    /// Its node, when the file is indexed.
    pub node: Option<NodeId>,
}

/// The check a shell command ran, as its event row keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCheck {
    /// The runner, by the name Cortex gives it (`cargo`, `pytest`).
    pub runner: Identifier,
    /// What it checks.
    pub kind: CommandKind,
}

/// One thing a task did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A file read.
    Read {
        /// When, in UTC milliseconds.
        at_ms: i64,
        /// The file.
        file: FileNode,
        /// The lines read, or none for the whole file.
        range: Option<LineRange>,
    },
    /// A file edited.
    Edit {
        /// When, in UTC milliseconds.
        at_ms: i64,
        /// The file.
        file: FileNode,
        /// The lines the edit covers after it, or none for the whole file, as for an edit only
        /// the file watcher saw.
        range: Option<LineRange>,
    },
    /// A search.
    Search {
        /// When, in UTC milliseconds.
        at_ms: i64,
        /// The files it found, as seen while the daemon ran; none in a state rebuilt from rows.
        hits: Vec<FileNode>,
    },
    /// A shell command.
    Run {
        /// When, in UTC milliseconds.
        at_ms: i64,
        /// The check its exit status vouches for, if any.
        check: Option<RunCheck>,
        /// Its exit code, when it finished in the foreground.
        exit_code: Option<i32>,
        /// The git restores it ran, when it exited 0, as seen while the daemon ran; none in a
        /// state rebuilt from rows.
        restores: Vec<Restore>,
        /// The counts its output printed, when it failed a check and the daemon read them while it
        /// ran; none in a state rebuilt from rows.
        counts: Option<CheckCounts>,
    },
}

impl Step {
    /// The files the step names.
    fn files_mut(&mut self) -> &mut [FileNode] {
        match self {
            Self::Read { file, .. } | Self::Edit { file, .. } => std::slice::from_mut(file),
            Self::Search { hits, .. } => hits,
            Self::Run { .. } => &mut [],
        }
    }
}

/// What the daemon saw of an event that its rows do not keep.
#[derive(Debug, Default)]
pub(crate) struct Unkept {
    /// The files a search found.
    pub(crate) hits: Vec<RepoPath>,
    /// The git restores a shell line ran.
    pub(crate) restores: Vec<Restore>,
    /// The counts a failed check's output printed.
    pub(crate) counts: Option<CheckCounts>,
    /// What an edited file held before the edit, when known.
    pub(crate) before: Option<FileContent>,
}

/// What one task did, as far as its rows show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskState {
    /// The task.
    task_id: Ulid,
    /// What it did, in order.
    steps: Vec<Step>,
    /// What each file it edited held before it first changed it, as far as that was seen.
    before: BTreeMap<RepoPath, FileContent>,
    /// The files it edited for which that was not seen, which a later edit cannot tell.
    unknown_before: BTreeSet<RepoPath>,
    /// How it ended, once it has.
    end: Option<TaskEnd>,
    /// How many of its rows the state was built from.
    rows: usize,
    /// How many bytes the paths of its searches' files hold.
    hit_bytes: usize,
    /// Whether it shows only part of what the task did.
    is_partial: bool,
    /// How many of its first steps were rebuilt from the task's rows, which keep no git restore a
    /// shell line ran: what those lines put back is not known. 0 when every step was seen.
    rebuilt_steps: usize,
}

impl TaskState {
    /// The state of a task that has done nothing yet.
    fn new(task_id: Ulid) -> Self {
        Self {
            task_id,
            steps: Vec::new(),
            before: BTreeMap::new(),
            unknown_before: BTreeSet::new(),
            end: None,
            rows: 0,
            hit_bytes: 0,
            is_partial: false,
            rebuilt_steps: 0,
        }
    }

    /// The task.
    #[must_use]
    pub fn task_id(&self) -> Ulid {
        self.task_id
    }

    /// What the task did, in the order its events were recorded.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// What each file the task edited held before the task first changed it, as far as that was
    /// seen while the daemon ran: from the index's report of the change, or from what the store
    /// held when the edit was recorded before the index saw it. A file created by the task held
    /// nothing; one the index does not read, contents not known. A file whose content was not
    /// seen when the task first changed it, because the state was rebuilt from its rows or the
    /// edit came too close to another's, is left out, and no later edit of it fills it in.
    #[must_use]
    pub fn before(&self) -> &BTreeMap<RepoPath, FileContent> {
        &self.before
    }

    /// How the task ended: `None` while it is still open.
    #[must_use]
    pub fn end(&self) -> Option<TaskEnd> {
        self.end
    }

    /// Whether the state shows only part of what the task did: the task had more than
    /// [`MAX_TASK_ROWS`] rows, of which the state shows the first, or its stored rows could not
    /// all be read back when the state was rebuilt, and it shows none of them.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.is_partial
    }

    /// How many of the task's first steps were rebuilt from its rows, after the daemon started
    /// again or the state was let go: the git restores their shell lines ran are not known, as
    /// rows keep none. Steps after them were seen as they happened. 0 when every step was seen.
    #[must_use]
    pub fn rebuilt_steps(&self) -> usize {
        self.rebuilt_steps
    }

    /// Notes `row`, the task's next, with what was seen of its event that the row does not keep.
    fn note(&mut self, row: &EventRecord, unkept: Unkept) {
        // INVARIANT: budgeted (a state holds the steps of at most MAX_TASK_ROWS rows)
        if self.rows == MAX_TASK_ROWS {
            self.is_partial = true;
            return;
        }
        self.rows += 1;
        let Some(mut step) = step_of(row, unkept.hits, unkept.restores, unkept.counts) else {
            return;
        };
        match &mut step {
            Step::Search { hits, .. } => self.hit_bytes = keep_within(hits, self.hit_bytes),
            Step::Edit { file, .. } => {
                let path = &file.path;
                if !self.before.contains_key(path) && !self.unknown_before.contains(path) {
                    match unkept.before {
                        Some(before) => {
                            self.before.insert(path.clone(), before);
                        }
                        None => {
                            self.unknown_before.insert(path.clone());
                        }
                    }
                }
            }
            Step::Read { .. } | Step::Run { .. } => {}
        }
        self.steps.push(step);
    }

    /// Gives each file of the steps that has no node the one `queries` finds for its path, if any,
    /// asking once per path.
    fn resolve(&mut self, queries: Queries<'_>) -> Result<(), StoreError> {
        let mut found: BTreeMap<RepoPath, Option<NodeId>> = BTreeMap::new();
        for file in self.steps.iter_mut().flat_map(Step::files_mut) {
            if file.node.is_some() {
                continue;
            }
            if let Some(node) = found.get(&file.path) {
                file.node = *node;
                continue;
            }
            let node = queries.file_node_at(&file.path)?;
            found.insert(file.path.clone(), node);
            file.node = node;
        }
        Ok(())
    }
}

/// The states of the tasks open at present, each with when it was last touched.
#[derive(Debug, Default)]
pub(crate) struct TaskStates {
    /// Each held state, by task, with the tick it was last touched at.
    held: BTreeMap<Ulid, (u64, TaskState)>,
    /// A count of touches, which orders them.
    ticks: u64,
}

impl TaskStates {
    /// Holds the state of the task `task_id`, rebuilt from the rows `queries` reads when it is not
    /// held.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub(crate) fn ensure(&mut self, queries: Queries<'_>, task_id: Ulid) -> Result<(), StoreError> {
        if !self.held.contains_key(&task_id) {
            let state = rebuild(queries, task_id)?;
            self.hold(state);
        }
        Ok(())
    }

    /// Notes `row`, just stored, in the state of its task, if held, with what was seen of its
    /// event that the row does not keep.
    pub(crate) fn note(&mut self, row: &EventRecord, unkept: Unkept) {
        let Some(task_id) = row.task_id else {
            return;
        };
        let Some((touched, state)) = self.held.get_mut(&task_id) else {
            return;
        };
        self.ticks += 1;
        *touched = self.ticks;
        state.note(row, unkept);
    }

    /// The state of the task `task_id`, which has ended as `end` says: the one held, else one
    /// rebuilt from its rows, each file given the node `queries` finds for its path.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub(crate) fn end(
        &mut self,
        queries: Queries<'_>,
        task_id: Ulid,
        end: TaskEnd,
    ) -> Result<TaskState, StoreError> {
        let mut state = match self.held.remove(&task_id) {
            Some((_, state)) => state,
            None => rebuild(queries, task_id)?,
        };
        state.end = Some(end);
        state.resolve(queries)?;
        Ok(state)
    }

    /// Drops every state, to be rebuilt from the store when its task is next touched.
    pub(crate) fn clear(&mut self) {
        self.held.clear();
    }

    /// Holds `state`, first dropping the state touched longest ago when there is no room.
    fn hold(&mut self, state: TaskState) {
        // INVARIANT: budgeted (at most MAX_TASK_STATES states are held)
        if self.held.len() >= MAX_TASK_STATES {
            let oldest = self
                .held
                .iter()
                .min_by_key(|(_, (touched, _))| *touched)
                .map(|(task_id, _)| *task_id);
            if let Some(task_id) = oldest {
                self.held.remove(&task_id);
            }
        }
        self.ticks += 1;
        self.held.insert(state.task_id, (self.ticks, state));
    }
}

/// The state of the task `task_id` as its stored rows show it, the first [`MAX_TASK_ROWS`] of them
/// read, and one more to tell whether there are more. Rows that cannot be read back, such as one of
/// a kind a later Cortex wrote, leave the state partial with none of them, rather than an error
/// that would stop every batch of events that touches the task.
///
/// # Errors
///
/// The store's errors but a stored value it cannot read back.
fn rebuild(queries: Queries<'_>, task_id: Ulid) -> Result<TaskState, StoreError> {
    let mut state = TaskState::new(task_id);
    match queries.events_of_task(task_id, MAX_TASK_ROWS + 1) {
        Ok(rows) => {
            for row in rows {
                state.note(&row.record, Unkept::default());
            }
        }
        Err(StoreError::Corrupt { .. }) => state.is_partial = true,
        Err(error) => return Err(error),
    }
    state.rebuilt_steps = state.steps.len();
    Ok(state)
}

/// Keeps the first of `hits` that fit, with the `held_bytes` a state's earlier searches' files
/// hold, within [`MAX_TASK_HIT_BYTES`], and returns the bytes held then: all of them once a file
/// did not fit, so that later searches keep none.
fn keep_within(hits: &mut Vec<FileNode>, held_bytes: usize) -> usize {
    let mut held_bytes = held_bytes;
    let mut kept = 0;
    for file in hits.iter() {
        let size_bytes = file.path.as_str().len();
        // INVARIANT: budgeted (a state's searches keep at most MAX_TASK_HIT_BYTES of found files)
        if held_bytes + size_bytes > MAX_TASK_HIT_BYTES {
            held_bytes = MAX_TASK_HIT_BYTES;
            break;
        }
        held_bytes += size_bytes;
        kept += 1;
    }
    hits.truncate(kept);
    held_bytes
}

/// The step `row` records, with what was seen of its event that the row does not keep: none for a
/// row of another kind, and for a read, edit or search that failed.
fn step_of(
    row: &EventRecord,
    hits: Vec<RepoPath>,
    restores: Vec<Restore>,
    counts: Option<CheckCounts>,
) -> Option<Step> {
    let at_ms = row.ts_ms;
    let has_failed = row.meta.is_success == Some(false);
    let file = || {
        row.path.clone().map(|path| FileNode {
            path,
            node: row.node_id,
        })
    };
    Some(match row.kind {
        EventKind::Read if !has_failed => Step::Read {
            at_ms,
            file: file()?,
            range: row.range,
        },
        EventKind::Edit if !has_failed => Step::Edit {
            at_ms,
            file: file()?,
            range: row.range,
        },
        EventKind::Search if !has_failed => Step::Search {
            at_ms,
            hits: hits
                .into_iter()
                .map(|path| FileNode { path, node: None })
                .collect(),
        },
        EventKind::Shell => Step::Run {
            at_ms,
            check: row
                .meta
                .runner
                .clone()
                .zip(row.meta.check)
                .map(|(runner, kind)| RunCheck { runner, kind }),
            exit_code: row.meta.exit_code,
            // A line's exit status vouches for its restores only when it is 0.
            restores: if row.meta.exit_code == Some(0) {
                restores
            } else {
                Vec::new()
            },
            counts,
        },
        _ => return None,
    })
}
