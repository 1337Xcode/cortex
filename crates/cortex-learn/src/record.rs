//! The recorder: one normalized event in, the rows the store keeps out.
//!
//! A prompt starts a task, first ending any task its session left open when a Stop never came;
//! the end of the answer and the end of the session end it. A subagent is a session of its own
//! whose parent is the session that started it, and its events belong to the parent's open task.
//! A tool call is recorded once it has run; the moment before it is answered and not kept. A shell
//! command keeps the runner and the kind of the check its exit status vouches for, never the
//! command. An event from a session the store does not hold registers that session first. Every
//! task that ends is marked `unknown`: telling success from failure needs the outcome rules, which
//! are not part of the record. Each row of a task is noted in the task's state as it is stored,
//! and the state is handed out when the task ends.

use std::collections::BTreeMap;

use cortex_core::{
    CommandKind, ContentHash, Event, EventKind, EventMeta, FileContent, HookKind, Identifier,
    LineRange, MAX_RANGES, PromptExcerpt, RepoPath, SessionStatus, TaskOutcome, ToolKind, Ulid,
    is_held_out, task_id_of_prompt,
};
use cortex_store::{EventRecord, SessionRecord, StoreError, TaskRecord, WriteTx};

use crate::activity::Activity;
use crate::commands::{Check, Cleaned, Commands, Ran, Restore, Told, clean_command};
use crate::task_state::{TaskEnd, TaskState, TaskStates, Unkept};
use crate::watcher_edits::WatcherEdits;

/// The reason Claude Code gives a session's end when the user cleared its conversation (`/clear`),
/// as its hooks documentation names it and a captured run showed.
const CLEARED_REASON: &str = "clear";

/// Most sessions the recorder remembers. Past that it forgets them all and reads each one again
/// from the store when its next event comes, so memory stays bounded however long it runs.
const MAX_REMEMBERED_SESSIONS: usize = 1_000;

/// Turns each event into rows, remembering per session what the store would otherwise be asked
/// on every event: that the session is stored with its parent, and its open task. It also
/// remembers what an edit the watcher saw is weighed against, sessions' recent work and the files
/// hooks reported edited lately, and holds the edits the watcher saw until a hook can no longer
/// claim them.
#[derive(Debug, Default)]
pub struct Recorder {
    /// Whether a task keeps the first characters of its prompt (`privacy.store_prompt_excerpt`).
    keeps_prompt_excerpt: bool,
    /// The share of tasks held out from Cortex's injections (`holdout.share`).
    holdout_share: f64,
    /// Sessions known to be stored, each with the session that started it, if any.
    parents: BTreeMap<Identifier, Option<Identifier>>,
    /// Each remembered session's open task, or `None` when it has none.
    open_tasks: BTreeMap<Identifier, Option<Ulid>>,
    /// What the events showed of sessions' work, for edits only the watcher saw.
    activity: Activity,
    /// What tells the checks a shell command ran.
    commands: Commands,
    /// What each open task did.
    states: TaskStates,
    /// The edits the watcher saw, held until a hook can no longer claim them.
    watcher_edits: WatcherEdits,
    /// When the index running now started, in UTC milliseconds, while one runs.
    index_started_ms: Option<i64>,
}

impl Recorder {
    /// A recorder that keeps prompt excerpts when `keeps_prompt_excerpt` says so, holds out
    /// `holdout_share` of the tasks it starts, and tells the checks shell commands ran with
    /// `commands`.
    #[must_use]
    pub fn new(keeps_prompt_excerpt: bool, holdout_share: f64, commands: Commands) -> Self {
        Self {
            keeps_prompt_excerpt,
            holdout_share,
            commands,
            ..Self::default()
        }
    }

    /// Records `event` in `transaction`, and returns the state of the task it ended, if it ended
    /// one: a prompt ends the task its session left open, the end of the answer or of the session
    /// the session's open task.
    ///
    /// # Errors
    ///
    /// Store errors. The recorder may then remember a session or a task the transaction did not
    /// keep, so after a failed transaction [`Recorder::forget_stored`] must be called. A state
    /// returned for an event of a transaction that is then rolled back describes a task the store
    /// still holds open, and is dropped with the transaction.
    pub fn record(
        &mut self,
        transaction: &mut WriteTx<'_>,
        event: &Event,
    ) -> Result<Option<TaskState>, StoreError> {
        let Some(kind) = kind_of(event) else {
            return Ok(None);
        };
        match event.hook {
            HookKind::SessionStart | HookKind::SubagentStart => {
                transaction.record_session(&session_of(event))?;
                // Stored now; its parent, which the store may keep from an earlier start, is read
                // again when needed.
                self.parents.remove(&event.session_id);
            }
            _ => self.register(transaction, event)?,
        }
        let ends_task = matches!(event.hook, HookKind::Stop | HookKind::SessionEnd);
        let (task, mut ended) = match event.hook {
            HookKind::Prompt => {
                let ended =
                    self.end_open_task(transaction, &event.session_id, event.ts_ms, TaskEnd::Left)?;
                (Some(self.start_task(transaction, event)?), ended)
            }
            // The end belongs to the task it ends, which ends once the end's row is in its state.
            _ if ends_task => (self.open_task(transaction, &event.session_id)?, None),
            _ => (self.task_of(transaction, event)?, None),
        };
        self.append_rows(transaction, (event, kind), task)?;
        self.activity.note(event, kind);
        if ends_task {
            // A session that ends with its answer unfinished leaves its task: the agent was
            // interrupted, cleared or stopped. A turn that ended on an error (a usage limit)
            // stops it, its stop naming the error.
            let end = match (event.hook, &event.end_reason) {
                (HookKind::Stop, None) => TaskEnd::Answered,
                (HookKind::Stop, Some(_)) => TaskEnd::Stopped,
                _ => TaskEnd::Left,
            };
            ended = self.end_open_task(transaction, &event.session_id, event.ts_ms, end)?;
        }
        if matches!(event.hook, HookKind::SessionEnd | HookKind::SubagentStop) {
            // A conversation the user cleared (`/clear`) set its work aside on purpose: no
            // session is handed what it left.
            let is_cleared = event
                .end_reason
                .as_ref()
                .is_some_and(|reason| reason.as_str() == CLEARED_REASON);
            let status = if is_cleared {
                SessionStatus::Cleared
            } else {
                SessionStatus::Ended
            };
            transaction.end_session(&event.session_id, (event.ts_ms, status))?;
            self.parents.remove(&event.session_id);
            self.open_tasks.remove(&event.session_id);
        }
        Ok(ended)
    }

    /// Notes that an index that started at `started_ms` runs, until [`Recorder::index_done`]: a
    /// hook's edit recorded meanwhile takes nothing from a row that index wrote for its file as what
    /// the file held before it, since the row may hold the edit's change already while the index's
    /// report of the change is not yet held.
    pub fn index_started(&mut self, started_ms: i64) {
        self.index_started_ms = Some(started_ms);
    }

    /// Notes that the index [`Recorder::index_started`] noted is done, the edits it found to be
    /// held next.
    pub fn index_done(&mut self) {
        self.index_started_ms = None;
    }

    /// Forgets what it took the store to hold, after a transaction it recorded into was rolled
    /// back: the sessions it took as stored, their open tasks, and the tasks' states, which are
    /// rebuilt from what the store does hold. What it saw of sessions' work stays: the events
    /// happened whether or not their rows were kept.
    pub fn forget_stored(&mut self) {
        self.parents.clear();
        self.open_tasks.clear();
        self.states.clear();
    }

    /// Stores `row` and notes it in its task's state with what was seen of its event that the row
    /// does not keep. A state that is not held is rebuilt from the task's rows first, so that the
    /// row is noted once.
    pub(crate) fn append(
        &mut self,
        transaction: &mut WriteTx<'_>,
        row: &EventRecord,
        unkept: Unkept,
    ) -> Result<(), StoreError> {
        if let Some(task_id) = row.task_id {
            self.states.ensure(transaction.queries(), task_id)?;
        }
        transaction.append_event(row)?;
        self.states.note(row, unkept);
        Ok(())
    }

    /// Stores the rows `event`, recorded as `kind`, is kept as, in `task`: one per range it covers,
    /// and for an edit of several files one more for each file after the first, without lines.
    fn append_rows(
        &mut self,
        transaction: &mut WriteTx<'_>,
        (event, kind): (&Event, EventKind),
        task: Option<Ulid>,
    ) -> Result<(), StoreError> {
        let target = event.tool_target.as_ref();
        let path = target.and_then(|target| target.path.clone());
        let node_id = match &path {
            Some(path) => transaction.queries().file_node_at(path)?,
            None => None,
        };
        let told = told_by(event, &self.commands);
        let mut meta = meta_of(event, &told);
        // A read keeps the short hash the index holds for its file: while the index still holds
        // that hash, the agent still holds the lines it read.
        if kind == EventKind::Read
            && meta.is_success != Some(false)
            && let Some(path) = &path
        {
            meta.content = transaction
                .queries()
                .file(path)?
                .and_then(|row| row.file.content_hash)
                .map(|hash| hash.short());
        }
        // What the rows do not keep goes with the first: a search or a shell command has one, and
        // an edit's file held one content before it.
        let mut unkept = Unkept {
            hits: target.map(|target| target.hits.clone()).unwrap_or_default(),
            restores: restores_of(told.ran),
            counts: event.tool_result.and_then(|result| result.counts),
            cleaned: cleaned_by(event, &self.commands),
            before: match &path {
                Some(path) => self.before_of(transaction, (event, kind), path)?,
                None => None,
            },
        };
        for range in ranges_of(event) {
            let row = EventRecord {
                task_id: task,
                session_id: Some(event.session_id.clone()),
                ts_ms: event.ts_ms,
                kind,
                node_id,
                path: path.clone(),
                range,
                meta: meta.clone(),
            };
            self.append(transaction, &row, std::mem::take(&mut unkept))?;
        }
        let more = target.map_or(&[][..], |target| target.more_paths.as_slice());
        for path in more.iter().filter(|_| kind == EventKind::Edit) {
            let row = EventRecord {
                task_id: task,
                session_id: Some(event.session_id.clone()),
                ts_ms: event.ts_ms,
                kind,
                node_id: transaction.queries().file_node_at(path)?,
                path: Some(path.clone()),
                range: None,
                meta: meta.clone(),
            };
            let unkept = Unkept {
                before: self.before_of(transaction, (event, kind), path)?,
                ..Unkept::default()
            };
            self.append(transaction, &row, unkept)?;
        }
        Ok(())
    }

    /// What the file of `event`, recorded as `kind`, held before it, when the event is a hook's
    /// edit: what the change the watcher saw of it nearby, still held, held before; else what the
    /// store holds for it, the index not having seen the change yet. Of the two, whichever came
    /// first tells. A task keeps it with the edit's step, which an edit that failed does not make.
    ///
    /// `None` for another event, and when the edit came too close to an earlier hook's edit of the
    /// same file to tell: the change held is that edit's, the watcher folding a second change
    /// within its hold into the first, or the index has not read the file since that edit, so the
    /// store holds what it held before that one. `None` too while an index runs that wrote the
    /// file's row: the row may hold this edit's change already, the index's report of it, with
    /// what the file held before, not yet held.
    fn before_of(
        &self,
        transaction: &WriteTx<'_>,
        (event, kind): (&Event, EventKind),
        path: &RepoPath,
    ) -> Result<Option<FileContent>, StoreError> {
        if kind != EventKind::Edit {
            return Ok(None);
        }
        if let Some((seen_ms, before)) = self.watcher_edits.held_near(path, event.ts_ms) {
            return Ok((!self.activity.is_reported(path, seen_ms)).then_some(before));
        }
        let stored = transaction.queries().file(path)?;
        let read_at_ms = stored.as_ref().map(|row| row.file.indexed_at_ms);
        let is_written_now = read_at_ms
            .zip(self.index_started_ms)
            .is_some_and(|(read_at_ms, started_ms)| read_at_ms >= started_ms);
        if is_written_now || self.activity.is_reported_since(path, read_at_ms) {
            return Ok(None);
        }
        Ok(Some(stored.map_or(FileContent::Absent, |row| {
            FileContent::of_stored(row.file.content_hash)
        })))
    }

    /// What the events showed of sessions' work.
    pub(crate) fn activity(&self) -> &Activity {
        &self.activity
    }

    /// The edits the watcher saw, held.
    pub(crate) fn watcher_edits(&self) -> &WatcherEdits {
        &self.watcher_edits
    }

    /// The edits the watcher saw, held, to change.
    pub(crate) fn watcher_edits_mut(&mut self) -> &mut WatcherEdits {
        &mut self.watcher_edits
    }

    /// Stores the event's session from what the event says, unless it is already stored.
    fn register(&mut self, transaction: &mut WriteTx<'_>, event: &Event) -> Result<(), StoreError> {
        if self.parents.contains_key(&event.session_id) {
            return Ok(());
        }
        let parent = if let Some(stored) = transaction.queries().session(&event.session_id)? {
            stored.record.parent_session_id
        } else {
            transaction.record_session(&session_of(event))?;
            event.parent_session_id.clone()
        };
        self.remember_parent(&event.session_id, parent);
        Ok(())
    }

    /// The session that started `session_id`, if any, remembered or read from the store.
    fn parent_of(
        &mut self,
        transaction: &WriteTx<'_>,
        session_id: &Identifier,
    ) -> Result<Option<Identifier>, StoreError> {
        if let Some(parent) = self.parents.get(session_id) {
            return Ok(parent.clone());
        }
        let parent = transaction
            .queries()
            .session(session_id)?
            .and_then(|stored| stored.record.parent_session_id);
        self.remember_parent(session_id, parent.clone());
        Ok(parent)
    }

    /// Remembers that `session_id` is stored, started by `parent`.
    fn remember_parent(&mut self, session_id: &Identifier, parent: Option<Identifier>) {
        if self.parents.len() >= MAX_REMEMBERED_SESSIONS {
            self.parents.clear();
        }
        self.parents.insert(session_id.clone(), parent);
    }

    /// The task the event belongs to: its session's open task, or for a subagent's event, the
    /// open task of the session that started the subagent, as the event names it or else as the
    /// store keeps it.
    fn task_of(
        &mut self,
        transaction: &WriteTx<'_>,
        event: &Event,
    ) -> Result<Option<Ulid>, StoreError> {
        match &event.parent_session_id {
            Some(parent) => self.open_task(transaction, parent),
            None => self.task_of_session(transaction, &event.session_id),
        }
    }

    /// The task an event of `session_id` belongs to: its open task, or for a subagent, the open
    /// task of the session that started it, as the store keeps it.
    pub(crate) fn task_of_session(
        &mut self,
        transaction: &WriteTx<'_>,
        session_id: &Identifier,
    ) -> Result<Option<Ulid>, StoreError> {
        let owner = self
            .parent_of(transaction, session_id)?
            .unwrap_or_else(|| session_id.clone());
        self.open_task(transaction, &owner)
    }

    /// The open task of `session_id`, remembered or read from the store.
    fn open_task(
        &mut self,
        transaction: &WriteTx<'_>,
        session_id: &Identifier,
    ) -> Result<Option<Ulid>, StoreError> {
        if let Some(open) = self.open_tasks.get(session_id) {
            return Ok(*open);
        }
        let open = transaction
            .queries()
            .open_task(session_id)?
            .map(|row| row.record.task_id);
        self.remember_open_task(session_id, open);
        Ok(open)
    }

    /// Remembers `open` as the open task of `session_id`.
    fn remember_open_task(&mut self, session_id: &Identifier, open: Option<Ulid>) {
        if self.open_tasks.len() >= MAX_REMEMBERED_SESSIONS {
            self.open_tasks.clear();
        }
        self.open_tasks.insert(session_id.clone(), open);
    }

    /// The session at work at `at_ms`: of those not ended before it, the one whose latest event
    /// came within `within_ms` before it; of two as late, the one whose id sorts first. `None`
    /// when none was at work then. A tool an agent calls in the middle of its turn, such as
    /// `cortex_note`, is taken to be that session's.
    #[must_use]
    pub fn session_at_work(&self, at_ms: i64, within_ms: i64) -> Option<Identifier> {
        self.activity.at_work(at_ms, (within_ms, 0))
    }

    /// The session at work at `at_ms` as [`Recorder::session_at_work`] finds it, when no other
    /// session had an event in the `within_ms` before it: what a request that names no session can
    /// be told apart by.
    #[must_use]
    pub fn sole_session_at_work(&self, at_ms: i64, within_ms: i64) -> Option<Identifier> {
        self.activity.sole_at_work(at_ms, (within_ms, 0))
    }

    /// Ends, as left, each task not ended whose session has shown no sign of work since
    /// `quiet_since_ms`, as of its last sign, and returns what each did, to learn from. An agent
    /// killed, crashed or interrupted with no stop and no session end leaves its task open; only
    /// an ended task leaves a handoff for the next session. Like [`Recorder::record`], a failed
    /// transaction needs [`Recorder::forget_stored`].
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub fn end_quiet_tasks(
        &mut self,
        transaction: &mut WriteTx<'_>,
        quiet_since_ms: i64,
    ) -> Result<Vec<TaskState>, StoreError> {
        let quiet = transaction.queries().quiet_open_tasks(quiet_since_ms)?;
        let mut ended = Vec::new();
        for task in quiet {
            ended.extend(self.end_open_task(
                transaction,
                &task.session_id,
                task.last_ms,
                TaskEnd::Left,
            )?);
        }
        Ok(ended)
    }

    /// Ends the open task of `session_id`, if it has one, as of `ended_at_ms` and in the way
    /// `end` says, and returns its state.
    fn end_open_task(
        &mut self,
        transaction: &mut WriteTx<'_>,
        session_id: &Identifier,
        ended_at_ms: i64,
        end: TaskEnd,
    ) -> Result<Option<TaskState>, StoreError> {
        let open = self.open_task(transaction, session_id)?;
        self.remember_open_task(session_id, None);
        let Some(task_id) = open else {
            return Ok(None);
        };
        transaction.end_task(task_id, ended_at_ms, TaskOutcome::Unknown)?;
        self.states
            .end(transaction.queries(), task_id, end)
            .map(Some)
    }

    /// Starts the task the prompt `event` opens and returns its id, made from the prompt's event,
    /// held out or not by that id.
    fn start_task(
        &mut self,
        transaction: &mut WriteTx<'_>,
        event: &Event,
    ) -> Result<Ulid, StoreError> {
        let task_id = task_id_of_prompt(event.event_id, event.ts_ms);
        let prompt = event.prompt_text.as_deref();
        transaction.start_task(&TaskRecord {
            task_id,
            session_id: event.session_id.clone(),
            started_at_ms: event.ts_ms,
            // INVARIANT: no-content-stored (a prompt is kept as its hash, and as its first
            // characters only when the user asks)
            prompt_hash: prompt.map(|text| ContentHash::of(text.as_bytes())),
            prompt_excerpt: prompt
                .filter(|_| self.keeps_prompt_excerpt)
                .map(PromptExcerpt::of),
            holdout: Some(is_held_out(task_id, self.holdout_share)),
        })?;
        self.remember_open_task(&event.session_id, Some(task_id));
        Ok(task_id)
    }
}

/// The kind of row an event is recorded as; `None` for the moment before a tool runs, which is
/// not kept.
fn kind_of(event: &Event) -> Option<EventKind> {
    Some(match event.hook {
        HookKind::PreTool => return None,
        HookKind::SessionStart => EventKind::SessionStart,
        HookKind::Prompt => EventKind::Prompt,
        HookKind::PostTool => match event.tool {
            Some(ToolKind::Read) => EventKind::Read,
            Some(ToolKind::Grep | ToolKind::Glob) => EventKind::Search,
            Some(ToolKind::Edit | ToolKind::Write | ToolKind::MultiEdit | ToolKind::ApplyPatch) => {
                EventKind::Edit
            }
            Some(ToolKind::Shell) => EventKind::Shell,
            Some(ToolKind::Other) | None => EventKind::Tool,
        },
        HookKind::Compact => EventKind::Compact,
        HookKind::Stop => EventKind::Stop,
        HookKind::SubagentStart => EventKind::SubagentStart,
        HookKind::SubagentStop => EventKind::SubagentStop,
        HookKind::SessionEnd => EventKind::SessionEnd,
    })
}

/// The session an event comes from, as the event describes it.
fn session_of(event: &Event) -> SessionRecord {
    SessionRecord {
        session_id: event.session_id.clone(),
        agent: event.agent,
        agent_version: event.agent_version.clone(),
        model: event.model.clone(),
        parent_session_id: event.parent_session_id.clone(),
        cwd: Some(event.cwd.clone()),
        transcript_path: event.transcript_path.clone(),
        started_at_ms: event.ts_ms,
    }
}

/// The line range of each row the event is recorded as: one row per range, or one row with no
/// range when there is none or more than [`MAX_RANGES`]: an edit of that many stretches is one of
/// the whole file.
fn ranges_of(event: &Event) -> Vec<Option<LineRange>> {
    let ranges = event
        .tool_target
        .as_ref()
        .map_or(&[][..], |target| target.ranges.as_slice());
    if ranges.is_empty() || ranges.len() > MAX_RANGES {
        return vec![None];
    }
    ranges.iter().copied().map(Some).collect()
}

/// What a handoff may keep of `event`'s shell command, cleaned by `commands`: only for a command
/// that exited otherwise than 0, so only a failed check's.
fn cleaned_by(event: &Event, commands: &Commands) -> Option<Cleaned> {
    let exit_code = event.tool_result.and_then(|result| result.exit_code)?;
    if exit_code == 0 {
        return None;
    }
    let command = event.tool_target.as_ref()?.command.as_ref()?;
    clean_command(commands, command, &event.repo_root, &event.cwd)
}

/// What `event`'s shell command tells of what it ran, as `commands` tells it: what its exit of 0
/// vouches for, and the check its exit otherwise is the failure of; nothing for another event.
fn told_by(event: &Event, commands: &Commands) -> Told {
    event
        .tool_target
        .as_ref()
        .and_then(|target| target.command.as_ref())
        .map(|command| commands.tell(command, &event.repo_root, &event.cwd))
        .unwrap_or_default()
}

/// The git restores of what a shell command `ran`.
fn restores_of(ran: Vec<Ran>) -> Vec<Restore> {
    ran.into_iter()
        .filter_map(|ran| match ran {
            Ran::Restore(restore) => Some(restore),
            Ran::Check(_) => None,
        })
        .collect()
}

/// The facts a row keeps besides its kind, path and lines; for a shell command, the check its exit
/// speaks for, as `told`: of an exit of 0, the check of what it vouches for that stands for the
/// line; of an exit otherwise, the check whose failure it is, or none when it may be another
/// command's.
fn meta_of(event: &Event, told: &Told) -> EventMeta {
    // INVARIANT: no-content-stored (a tool's command, search pattern and output never reach the
    // row: only how it ended, how much it printed, and the runner and kind of the check it ran)
    let result = event.tool_result;
    let exit_code = result.and_then(|result| result.exit_code);
    let check = match exit_code {
        Some(code) if code != 0 => told.failure,
        _ => told
            .ran
            .iter()
            .filter_map(|ran| match ran {
                Ran::Check(check) => Some(*check),
                Ran::Restore(_) => None,
            })
            .min_by_key(|check| rank(check.kind)),
    };
    EventMeta {
        source: None,
        tool: event.tool,
        exit_code,
        is_success: result.map(|result| result.is_success),
        is_interrupted: result.map(|result| result.is_interrupted),
        output_bytes: result.and_then(|result| result.output_bytes),
        start_source: event.source,
        compact_trigger: event.compact_trigger,
        end_reason: event.end_reason.clone(),
        subagent_type: event.subagent_type.clone(),
        // Every runner's name is an identifier (a test of the runners says so), so none is lost.
        runner: check.and_then(|Check { runner, .. }| Identifier::new(runner).ok()),
        check: check.map(|check| check.kind),
        content: None,
        cut_after: event
            .tool_target
            .as_ref()
            .and_then(|target| target.cut_after),
    }
}

/// Which check stands for a line that ran several, the lowest first: tests, then a build, a type
/// check, a lint, so that the one kept tells the most about the edits before it (a lint counts for
/// no outcome). Of two of a kind, the first in the line stands.
fn rank(kind: CommandKind) -> u8 {
    match kind {
        CommandKind::Test => 0,
        CommandKind::Build => 1,
        CommandKind::TypeCheck => 2,
        CommandKind::Lint => 3,
    }
}
