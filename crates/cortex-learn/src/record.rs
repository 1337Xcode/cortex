//! The recorder: one normalized event in, the rows the store keeps out.
//!
//! A prompt starts a task, first ending any task its session left open when a Stop never came;
//! the end of the answer and the end of the session end it. A subagent is a session of its own
//! whose parent is the session that started it, and its events belong to the parent's open task.
//! A tool call is recorded once it has run; the moment before it is answered and not kept. An
//! event from a session the store does not hold registers that session first. Every task that
//! ends is marked `unknown`: telling success from failure needs the outcome rules, which are not
//! part of the record.

use std::collections::BTreeMap;

use cortex_core::{
    ContentHash, Event, EventKind, EventMeta, HookKind, Identifier, LineRange, MAX_RANGES,
    PromptExcerpt, TaskOutcome, ToolKind, Ulid,
};
use cortex_store::{EventRecord, SessionRecord, StoreError, TaskRecord, WriteTx};

/// Most sessions the recorder remembers. Past that it forgets them all and reads each one again
/// from the store when its next event comes, so memory stays bounded however long it runs.
const MAX_REMEMBERED_SESSIONS: usize = 1_000;

/// Turns each event into rows, remembering per session what the store would otherwise be asked
/// on every event: that the session is stored with its parent, and its open task.
#[derive(Debug, Default)]
pub struct Recorder {
    /// Whether a task keeps the first characters of its prompt (`privacy.store_prompt_excerpt`).
    keeps_prompt_excerpt: bool,
    /// Sessions known to be stored, each with the session that started it, if any.
    parents: BTreeMap<Identifier, Option<Identifier>>,
    /// Each remembered session's open task, or `None` when it has none.
    open_tasks: BTreeMap<Identifier, Option<Ulid>>,
}

impl Recorder {
    /// A recorder that keeps prompt excerpts when `keeps_prompt_excerpt` says so.
    #[must_use]
    pub fn new(keeps_prompt_excerpt: bool) -> Self {
        Self {
            keeps_prompt_excerpt,
            ..Self::default()
        }
    }

    /// Records `event` in `transaction`.
    ///
    /// # Errors
    ///
    /// Store errors. The recorder may then remember a session or a task the transaction did not
    /// keep, so after a failed transaction it must be replaced with a new one.
    pub fn record(
        &mut self,
        transaction: &mut WriteTx<'_>,
        event: &Event,
    ) -> Result<(), StoreError> {
        let Some(kind) = kind_of(event) else {
            return Ok(());
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
        let task = match event.hook {
            HookKind::Prompt => {
                self.end_open_task(transaction, &event.session_id, event.ts_ms)?;
                Some(self.start_task(transaction, event)?)
            }
            HookKind::Stop | HookKind::SessionEnd => {
                self.end_open_task(transaction, &event.session_id, event.ts_ms)?
            }
            _ => self.task_of(transaction, event)?,
        };
        let node_id = match event
            .tool_target
            .as_ref()
            .and_then(|target| target.path.as_ref())
        {
            Some(path) => transaction.queries().file_node_at(path)?,
            None => None,
        };
        for range in ranges_of(event) {
            transaction.append_event(&EventRecord {
                task_id: task,
                session_id: Some(event.session_id.clone()),
                ts_ms: event.ts_ms,
                kind,
                node_id,
                path: event
                    .tool_target
                    .as_ref()
                    .and_then(|target| target.path.clone()),
                range,
                meta: meta_of(event),
            })?;
        }
        if matches!(event.hook, HookKind::SessionEnd | HookKind::SubagentStop) {
            transaction.end_session(&event.session_id, event.ts_ms)?;
            self.parents.remove(&event.session_id);
            self.open_tasks.remove(&event.session_id);
        }
        Ok(())
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
        let owner = match &event.parent_session_id {
            Some(parent) => parent.clone(),
            None => self
                .parent_of(transaction, &event.session_id)?
                .unwrap_or_else(|| event.session_id.clone()),
        };
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

    /// Ends the open task of `session_id`, if it has one, as of `ended_at_ms`, and returns it.
    fn end_open_task(
        &mut self,
        transaction: &mut WriteTx<'_>,
        session_id: &Identifier,
        ended_at_ms: i64,
    ) -> Result<Option<Ulid>, StoreError> {
        let open = self.open_task(transaction, session_id)?;
        if let Some(task) = open {
            transaction.end_task(task, ended_at_ms, TaskOutcome::Unknown)?;
        }
        self.remember_open_task(session_id, None);
        Ok(open)
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

/// The facts a row keeps besides its kind, path and lines.
fn meta_of(event: &Event) -> EventMeta {
    // INVARIANT: no-content-stored (a tool's command, search pattern and output never reach the
    // row: only how it ended and how much it printed)
    let result = event.tool_result;
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
    }
}
