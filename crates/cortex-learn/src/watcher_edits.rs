//! Edits the file watcher saw that no hook reported: an agent without hooks, an editor, a person.
//!
//! The watcher reports a change after 200 ms of quiet; the hook of an agent that made the edit
//! reports it within moments of the tool ending. So each edit the watcher saw is held for
//! [`HOLD_MS`], and a hook's edit of the same file at most that far from it, before or after, is
//! the same edit, already recorded. An edit no hook claims is recorded for the session that was
//! at work at the time: of those not ended before the edit, the one with the latest event from
//! [`ATTRIBUTION_WINDOW_MS`] before it to [`HOLD_MS`] after it, a subagent's events counting for
//! the session that started it; or for no session, as the watcher's.
//!
//! Each held edit also keeps what its file held before, as the index found it. The task an edit is
//! recorded in keeps that as the file's content before the task: a hook's edit takes it from the
//! held edit of its file, the watcher's edit from itself.

use std::collections::BTreeMap;

use cortex_core::{EventKind, EventMeta, EventSource, FileContent, Identifier, RepoPath};
use cortex_store::{EventRecord, StoreError, WriteTx};

use crate::Recorder;
use crate::task_state::Unkept;

/// How long an edit the watcher saw waits for a hook to report it, and how far apart a hook's
/// edit and the watcher's may be to count as one, in milliseconds.
pub const HOLD_MS: i64 = 2_000;

/// How much older than an edit no hook reported a session's latest event may be for the edit to
/// count as that session's, in milliseconds.
pub const ATTRIBUTION_WINDOW_MS: i64 = 30_000;

/// An edit the watcher saw, as it is held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldEdit {
    /// The file.
    pub path: RepoPath,
    /// When the watcher first saw it change while it was held, in UTC milliseconds.
    pub seen_ms: i64,
    /// What the file held before that change, as the index found it.
    pub before: FileContent,
}

/// The edits the watcher saw, each held until [`HOLD_MS`] after it was seen. The watcher reports
/// a change to everything instead of more than a few hundred paths at once, so few are held.
#[derive(Debug, Default)]
pub(crate) struct WatcherEdits {
    /// Each file held, with when the watcher first saw it change while it was held and what it held
    /// before then.
    held: BTreeMap<RepoPath, (i64, FileContent)>,
}

impl WatcherEdits {
    /// Holds each of `edits`, a file and what it held before, seen changed at `seen_ms`; a file
    /// already held keeps its earlier time and content.
    pub(crate) fn hold(
        &mut self,
        edits: impl IntoIterator<Item = (RepoPath, FileContent)>,
        seen_ms: i64,
    ) {
        for (path, before) in edits {
            self.held.entry(path).or_insert((seen_ms, before));
        }
    }

    /// When the next held edit is due, in UTC milliseconds; `None` when nothing is held.
    pub(crate) fn next_due_ms(&self) -> Option<i64> {
        self.held
            .values()
            .map(|(seen_ms, _)| *seen_ms)
            .min()
            .map(|seen_ms| seen_ms.saturating_add(HOLD_MS))
    }

    /// Takes the edits due by `now_ms`: the earliest first, and those seen at once in path order.
    pub(crate) fn take_due(&mut self, now_ms: i64) -> Vec<HeldEdit> {
        let mut due: Vec<HeldEdit> = self
            .held
            .iter()
            .filter(|(_, (seen_ms, _))| seen_ms.saturating_add(HOLD_MS) <= now_ms)
            .map(|(path, (seen_ms, before))| HeldEdit {
                path: path.clone(),
                seen_ms: *seen_ms,
                before: *before,
            })
            .collect();
        for edit in &due {
            self.held.remove(&edit.path);
        }
        due.sort_by(|one, other| {
            one.seen_ms
                .cmp(&other.seen_ms)
                .then_with(|| one.path.cmp(&other.path))
        });
        due
    }

    /// What `path` held before the change of it held now, when the watcher saw that change at most
    /// [`HOLD_MS`] before or after `at_ms`.
    pub(crate) fn before_near(&self, path: &RepoPath, at_ms: i64) -> Option<FileContent> {
        let (seen_ms, before) = self.held.get(path)?;
        (seen_ms.abs_diff(at_ms) <= HOLD_MS.unsigned_abs()).then_some(*before)
    }
}

/// What became of an edit the watcher saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatcherEdit {
    /// A hook reported it: nothing more is recorded.
    Reported,
    /// It was recorded for this session.
    Session(Identifier),
    /// It was recorded for no session: none had an event recent enough.
    Nobody,
}

impl Recorder {
    /// Holds each of `edits`, a file the index found edited and what it held before, seen changed
    /// at `seen_ms`, until a hook can no longer claim it; a file already held keeps its earlier
    /// time and content.
    pub fn hold_watcher_edits(
        &mut self,
        edits: impl IntoIterator<Item = (RepoPath, FileContent)>,
        seen_ms: i64,
    ) {
        self.watcher_edits_mut().hold(edits, seen_ms);
    }

    /// When the next held edit is due, in UTC milliseconds; `None` when nothing is held.
    #[must_use]
    pub fn next_watcher_edit_due_ms(&self) -> Option<i64> {
        self.watcher_edits().next_due_ms()
    }

    /// Takes the held edits due by `now_ms`, to be recorded: the earliest first, and those seen at
    /// once in path order.
    pub fn take_due_watcher_edits(&mut self, now_ms: i64) -> Vec<HeldEdit> {
        self.watcher_edits_mut().take_due(now_ms)
    }

    /// Records `edit`, an edit the watcher saw, unless a hook reported it, for the session at work
    /// then, as the module documentation says, in that session's task, or for no session. The row
    /// is an edit with no line range whose source is the watcher; the task keeps what the file held
    /// before, unless it holds an earlier content for it.
    ///
    /// # Errors
    ///
    /// Store errors, after which [`Recorder::forget_stored`] must be called, as after
    /// [`Recorder::record`].
    pub fn record_watcher_edit(
        &mut self,
        transaction: &mut WriteTx<'_>,
        edit: &HeldEdit,
    ) -> Result<WatcherEdit, StoreError> {
        if self.activity().is_reported(&edit.path, edit.seen_ms) {
            return Ok(WatcherEdit::Reported);
        }
        let session = self.activity().owner_of(edit.seen_ms);
        let task = match &session {
            Some(session_id) => self.task_of_session(transaction, session_id)?,
            None => None,
        };
        let node_id = transaction.queries().file_node_at(&edit.path)?;
        let row = EventRecord {
            task_id: task,
            session_id: session.clone(),
            ts_ms: edit.seen_ms,
            kind: EventKind::Edit,
            node_id,
            path: Some(edit.path.clone()),
            range: None,
            meta: EventMeta {
                source: Some(EventSource::Watcher),
                ..EventMeta::default()
            },
        };
        let unkept = Unkept {
            before: Some(edit.before),
            ..Unkept::default()
        };
        self.append(transaction, &row, unkept)?;
        Ok(session.map_or(WatcherEdit::Nobody, WatcherEdit::Session))
    }
}
