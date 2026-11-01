//! Edits the file watcher saw that no hook reported: an agent without hooks, an editor, a person.
//!
//! The watcher reports a change after 200 ms of quiet; the hook of an agent that made the edit
//! reports it within moments of the tool ending. So each edit the watcher saw is held for
//! [`HOLD_MS`], and a hook's edit of the same file at most that far from it, before or after, is
//! the same edit, already recorded. An edit no hook claims is recorded for the live session whose
//! latest event is the most recent and at most [`ATTRIBUTION_WINDOW_MS`] older than the edit, or
//! for no session, as the watcher's.

use std::collections::BTreeMap;

use cortex_core::{EventKind, EventMeta, EventSource, Identifier, RepoPath};
use cortex_store::{EventRecord, StoreError, WriteTx};

use crate::Recorder;

/// How long an edit the watcher saw waits for a hook to report it, and how far apart a hook's
/// edit and the watcher's may be to count as one, in milliseconds.
pub const HOLD_MS: i64 = 2_000;

/// How much older than an edit no hook reported a session's latest event may be for the edit to
/// count as that session's, in milliseconds.
pub const ATTRIBUTION_WINDOW_MS: i64 = 30_000;

/// The edits the watcher saw, each held until [`HOLD_MS`] after it was seen. The watcher reports
/// a change to everything instead of more than a few hundred paths at once, so few are held.
#[derive(Debug, Default)]
pub struct WatcherEdits {
    /// Each file held, with when the watcher first saw it change while it was held.
    held: BTreeMap<RepoPath, i64>,
}

impl WatcherEdits {
    /// Holds each of `paths`, seen changed at `seen_ms`; a file already held keeps its earlier time.
    pub fn hold(&mut self, paths: impl IntoIterator<Item = RepoPath>, seen_ms: i64) {
        for path in paths {
            self.held.entry(path).or_insert(seen_ms);
        }
    }

    /// When the next held edit is due, in UTC milliseconds; `None` when nothing is held.
    #[must_use]
    pub fn next_due_ms(&self) -> Option<i64> {
        self.held
            .values()
            .min()
            .map(|seen_ms| seen_ms.saturating_add(HOLD_MS))
    }

    /// Takes the edits due by `now_ms`, each with when it was seen: the earliest first, and those
    /// seen at once in path order.
    pub fn take_due(&mut self, now_ms: i64) -> Vec<(RepoPath, i64)> {
        let mut due: Vec<(RepoPath, i64)> = self
            .held
            .iter()
            .filter(|(_, seen_ms)| seen_ms.saturating_add(HOLD_MS) <= now_ms)
            .map(|(path, seen_ms)| (path.clone(), *seen_ms))
            .collect();
        for (path, _) in &due {
            self.held.remove(path);
        }
        due.sort_by(|(one, one_ms), (other, other_ms)| {
            one_ms.cmp(other_ms).then_with(|| one.cmp(other))
        });
        due
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
    /// Records the edit of `path` the watcher saw at `seen_ms`, unless a hook reported it, for the
    /// live session whose latest event is the most recent and at most [`ATTRIBUTION_WINDOW_MS`]
    /// older than it, in that session's task, or for no session. The row is an edit with no line
    /// range whose source is the watcher.
    ///
    /// # Errors
    ///
    /// Store errors, after which the recorder must be replaced, as after [`Recorder::record`].
    pub fn record_watcher_edit(
        &mut self,
        transaction: &mut WriteTx<'_>,
        path: &RepoPath,
        seen_ms: i64,
    ) -> Result<WatcherEdit, StoreError> {
        if self.has_reported_edit(path, seen_ms, HOLD_MS) {
            return Ok(WatcherEdit::Reported);
        }
        let session = self.latest_session(seen_ms.saturating_sub(ATTRIBUTION_WINDOW_MS));
        let task = match &session {
            Some(session_id) => self.task_of_session(transaction, session_id)?,
            None => None,
        };
        let node_id = transaction.queries().file_node_at(path)?;
        transaction.append_event(&EventRecord {
            task_id: task,
            session_id: session.clone(),
            ts_ms: seen_ms,
            kind: EventKind::Edit,
            node_id,
            path: Some(path.clone()),
            range: None,
            meta: EventMeta {
                source: Some(EventSource::Watcher),
                ..EventMeta::default()
            },
        })?;
        Ok(session.map_or(WatcherEdit::Nobody, WatcherEdit::Session))
    }
}
