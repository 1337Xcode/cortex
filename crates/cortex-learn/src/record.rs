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
    LineRange, MAX_RANGES, PromptExcerpt, TaskOutcome, ToolKind, Ulid,
};
use cortex_store::{EventRecord, SessionRecord, StoreError, TaskRecord, WriteTx};

use crate::activity::Activity;
use crate::commands::{Check, Commands, Ran, Restore};
use crate::task_state::{TaskEnd, TaskState, TaskStates, Unkept};
use crate::watcher_edits::WatcherEdits;

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
}

impl Recorder {
    /// A recorder that keeps prompt excerpts when `keeps_prompt_excerpt` says so, and tells the
    /// checks shell commands ran with `commands`.
    #[must_use]
    pub fn new(keeps_prompt_excerpt: bool, commands: Commands) -> Self {
        Self {
            keeps_prompt_excerpt,
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
            // interrupted, cleared or stopped.
            let end = if event.hook == HookKind::Stop {
                TaskEnd::Answered
            } else {
                TaskEnd::Left
            };
            ended = self.end_open_task(transaction, &event.session_id, event.ts_ms, end)?;
        }
        if matches!(event.hook, HookKind::SessionEnd | HookKind::SubagentStop) {
            transaction.end_session(&event.session_id, event.ts_ms)?;
            self.parents.remove(&event.session_id);
            self.open_tasks.remove(&event.session_id);
        }
        Ok(ended)
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

    /// Stores the rows `event`, recorded as `kind`, is kept as, in `task`: one per range it covers.
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
        let ran = ran_by(event, &self.commands);
        let meta = meta_of(event, &ran);
        // What the rows do not keep goes with the first: a search or a shell command has one, and
        // an edit's file held one content before it.
        let mut unkept = Unkept {
            hits: target.map(|target| target.hits.clone()).unwrap_or_default(),
            restores: restores_of(ran),
            before: self.before_of(transaction, (event, kind))?,
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
        Ok(())
    }

    /// What the file of `event`, recorded as `kind`, held before it, when the event is a hook's
    /// edit: what the change the watcher saw of it nearby, still held, held before; else what the
    /// store holds for it, the index not having seen the change yet. Of the two, whichever came
    /// first tells. `None` for another event. A task keeps it with the edit's step, which an edit
    /// that failed does not make.
    fn before_of(
        &self,
        transaction: &WriteTx<'_>,
        (event, kind): (&Event, EventKind),
    ) -> Result<Option<FileContent>, StoreError> {
        let path = event
            .tool_target
            .as_ref()
            .and_then(|target| target.path.as_ref());
        let Some(path) = path.filter(|_| kind == EventKind::Edit) else {
            return Ok(None);
        };
        if let Some(before) = self.watcher_edits.before_near(path, event.ts_ms) {
            return Ok(Some(before));
        }
        let stored = transaction.queries().file(path)?;
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

    /// Starts the task the prompt `event` opens and returns its id, made from the prompt's event.
    fn start_task(
        &mut self,
        transaction: &mut WriteTx<'_>,
        event: &Event,
    ) -> Result<Ulid, StoreError> {
        let task_id = Ulid::from_seed(
            u64::try_from(event.ts_ms).unwrap_or(0),
            event.event_id.to_string().as_bytes(),
        );
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

/// What `event`'s shell command ran that its exit status vouches for, as `commands` tells it;
/// nothing for another event.
fn ran_by(event: &Event, commands: &Commands) -> Vec<Ran> {
    event
        .tool_target
        .as_ref()
        .and_then(|target| target.command.as_ref())
        .map(|command| commands.recognise(command, &event.repo_root, &event.cwd))
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

/// The facts a row keeps besides its kind, path and lines; for a shell command, the check of what
/// it `ran` that stands for the line.
fn meta_of(event: &Event, ran: &[Ran]) -> EventMeta {
    // INVARIANT: no-content-stored (a tool's command, search pattern and output never reach the
    // row: only how it ended, how much it printed, and the runner and kind of the check it ran)
    let result = event.tool_result;
    let check = ran
        .iter()
        .filter_map(|ran| match ran {
            Ran::Check(check) => Some(*check),
            Ran::Restore(_) => None,
        })
        .min_by_key(|check| rank(check.kind));
    EventMeta {
        source: None,
        tool: event.tool,
        exit_code: result.and_then(|result| result.exit_code),
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
