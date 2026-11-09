//! What the recorder saw of agents' work that tells whose an edit only the file watcher saw is:
//! when each session was at work lately, its subagents' work counted as its own, when it ended, and
//! when hooks reported each file edited. It is kept from the events themselves, so it holds whether
//! or not their rows were kept, and an edit is weighed only against events from 30 s before it to
//! 2 s after it, so the answer does not depend on the order events arrived in.

use std::collections::{BTreeMap, BTreeSet};

use cortex_core::{Event, EventKind, HookKind, Identifier, RepoPath, ToolTarget};

use crate::watcher_edits::{ATTRIBUTION_WINDOW_MS, HOLD_MS};

/// How long an event is remembered after the newest of its session, or of its file, in
/// milliseconds: an edit, weighed once its hold is over, looks back 30 s from when it was seen.
const KEEP_MS: i64 = ATTRIBUTION_WINDOW_MS + 2 * HOLD_MS;

/// Most sessions, and most files, remembered at once. Past that, those whose newest event is
/// older than [`KEEP_MS`] before the event being noted are forgotten.
const MAX_REMEMBERED: usize = 1_000;

/// One session's work, as the edits the watcher saw are weighed against it.
#[derive(Debug, Default)]
struct Session {
    /// When its recent events happened, its subagents' included, in UTC milliseconds.
    times: BTreeSet<i64>,
    /// When it ended, if it has and has not started again since.
    ended_ms: Option<i64>,
}

/// See the module documentation.
#[derive(Debug, Default)]
pub(crate) struct Activity {
    /// Each session at work lately, by its own id, or for a subagent's events by the id of the
    /// session that started the subagent.
    sessions: BTreeMap<Identifier, Session>,
    /// When hooks reported each file edited lately.
    reported: BTreeMap<RepoPath, BTreeSet<i64>>,
}

impl Activity {
    /// Notes `event`, recorded as `kind`: its time for its session, or its starter's, the end of a
    /// session or its start again, and the file a hook reported edited.
    pub(crate) fn note(&mut self, event: &Event, kind: EventKind) {
        let since_ms = event.ts_ms.saturating_sub(KEEP_MS);
        let owner = event
            .parent_session_id
            .as_ref()
            .unwrap_or(&event.session_id);
        if self.sessions.len() >= MAX_REMEMBERED && !self.sessions.contains_key(owner) {
            self.sessions
                .retain(|_, session| session.times.last().is_some_and(|ts_ms| *ts_ms >= since_ms));
        }
        let session = self.sessions.entry(owner.clone()).or_default();
        session.times.insert(event.ts_ms);
        session.times = session.times.split_off(&since_ms);
        if event.parent_session_id.is_none() {
            match event.hook {
                HookKind::SessionEnd => {
                    let ended_ms = session
                        .ended_ms
                        .map_or(event.ts_ms, |ended| ended.min(event.ts_ms));
                    session.ended_ms = Some(ended_ms);
                }
                HookKind::SessionStart
                    if session.ended_ms.is_some_and(|ended| ended <= event.ts_ms) =>
                {
                    session.ended_ms = None;
                }
                _ => {}
            }
        }
        let edited = event.tool_target.iter().flat_map(ToolTarget::paths);
        // An edit that failed changed nothing, so it claims no change the watcher sees in its file.
        let is_made = event
            .tool_result
            .as_ref()
            .is_none_or(|result| result.is_success);
        if kind != EventKind::Edit || !is_made {
            return;
        }
        for path in edited {
            if self.reported.len() >= MAX_REMEMBERED && !self.reported.contains_key(path) {
                self.reported
                    .retain(|_, times| times.last().is_some_and(|ts_ms| *ts_ms >= since_ms));
            }
            let times = self.reported.entry(path.clone()).or_default();
            times.insert(event.ts_ms);
            *times = times.split_off(&since_ms);
        }
    }

    /// Whether a hook reported an edit of `path` at most [`HOLD_MS`] before or after `seen_ms`.
    pub(crate) fn is_reported(&self, path: &RepoPath, seen_ms: i64) -> bool {
        let around = seen_ms.saturating_sub(HOLD_MS)..=seen_ms.saturating_add(HOLD_MS);
        self.reported
            .get(path)
            .is_some_and(|times| times.range(around).next().is_some())
    }

    /// Whether a hook reported an edit of `path` at or after `read_at_ms`, when the index last
    /// read the file, or at all for a file the index never read: a change the index may not have
    /// read yet.
    pub(crate) fn is_reported_since(&self, path: &RepoPath, read_at_ms: Option<i64>) -> bool {
        self.reported
            .get(path)
            .is_some_and(|times| match read_at_ms {
                Some(read_at_ms) => times.range(read_at_ms..).next().is_some(),
                None => !times.is_empty(),
            })
    }

    /// The session most likely to have made an edit seen at `seen_ms`: of those not ended before
    /// it, the one with the latest event from [`ATTRIBUTION_WINDOW_MS`] before it to [`HOLD_MS`]
    /// after it; of two as late, the one whose id sorts first. `None` when none was at work then.
    pub(crate) fn owner_of(&self, seen_ms: i64) -> Option<Identifier> {
        self.at_work(seen_ms, (ATTRIBUTION_WINDOW_MS, HOLD_MS))
    }

    /// Of the sessions not ended before `at_ms`, the one with the latest event from `before_ms`
    /// before it to `after_ms` after it; of two as late, the one whose id sorts first. `None` when
    /// none was at work then.
    pub(crate) fn at_work(
        &self,
        at_ms: i64,
        (before_ms, after_ms): (i64, i64),
    ) -> Option<Identifier> {
        let seen_ms = at_ms;
        let around = seen_ms.saturating_sub(before_ms)..=seen_ms.saturating_add(after_ms);
        self.sessions
            .iter()
            .filter(|(_, session)| session.ended_ms.is_none_or(|ended| ended >= seen_ms))
            .filter_map(|(session_id, session)| {
                let latest = session.times.range(around.clone()).next_back()?;
                Some((session_id, *latest))
            })
            .max_by(|(one, one_ms), (other, other_ms)| {
                one_ms.cmp(other_ms).then_with(|| other.cmp(one))
            })
            .map(|(session_id, _)| session_id.clone())
    }
}
