//! Coordination across the sessions working in one repository, as the daemon answers it: which
//! other sessions an edit concerns, the lines the editing session is told at once and the notice
//! each concerned session is told at its next hook, and the notices a session is given ahead of the
//! rest of an answer. Positions come from the store, as the writer kept them; how files are linked
//! from the store's edges; the rules and the words from `cortex-coord`. Nothing here writes: what
//! to queue, tell and mark delivered goes to the writer with the answer.

use std::collections::BTreeSet;

use cortex_coord::{EditAt, Link, Notice, Party, Position, Thresholds, Timing, concerned};
use cortex_core::{
    Agent, CoordinationConfig, EdgeType, Event, Identifier, NoticeSubject, RepoPath,
    estimate_tokens,
};
use cortex_store::{Coupling, NodeId, NoticeRecord, PositionRow, Queries, SeenSession};

use super::{Answer, Service, task_of_answer};
use crate::claim::Claim;
use crate::given::{Given, Notices, text_record};
use crate::noticed::{Giving, Noticed};

/// What an edit's coordination found, before it is given.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Coordination {
    /// The lines the editing session could be told, those that act first leading.
    pub(crate) lines: Vec<Notice>,
    /// The notices for the sessions it concerns.
    pub(crate) queued: Vec<NoticeRecord>,
}

/// One concern found, with whether it acts first.
type Line = (bool, Notice);

/// The other sessions around an edit: their positions outside the editing session's own family,
/// the nodes those are on, and what the store knows of each session.
struct Around {
    /// The positions the other sessions hold.
    held: Vec<Position<Identifier, NodeId>>,
    /// The nodes they hold, each once, in order.
    nodes: Vec<NodeId>,
    /// What the store knows of every session holding one, and of the editing session.
    seen: Vec<SeenSession>,
}

/// What an edit's coordination found so far: the lines its session could be told and the
/// notices for the others.
#[derive(Default)]
struct Found {
    /// The lines, with whether each acts first, in the order found.
    lines: Vec<Line>,
    /// The notices.
    queued: Vec<NoticeRecord>,
}

impl Around {
    /// What surrounds `event`, an edit, within the longest hold of `timing`; none when no other
    /// session holds a position.
    ///
    /// # Errors
    ///
    /// The store's errors.
    fn read(
        queries: Queries<'_>,
        event: &Event,
        timing: &Timing,
    ) -> Result<Option<Self>, cortex_store::StoreError> {
        let positions = queries.positions_since(event.ts_ms.saturating_sub(timing.longest_ms()))?;
        let mut sessions: Vec<Identifier> = positions
            .iter()
            .map(|row| row.record.session_id.clone())
            .filter(|session| *session != event.session_id)
            .collect();
        if sessions.is_empty() {
            return Ok(None);
        }
        sessions.push(event.session_id.clone());
        sessions.sort_unstable();
        sessions.dedup();
        let seen = queries.sessions_seen(&sessions)?;
        let family = family_of(event, &seen);
        let held: Vec<Position<Identifier, NodeId>> = positions
            .into_iter()
            .filter(|row| !family.contains(&row.record.session_id))
            .map(rule_of)
            .collect();
        let mut nodes: Vec<NodeId> = held.iter().map(|position| position.node).collect();
        nodes.sort_unstable();
        nodes.dedup();
        Ok(Some(Self { held, nodes, seen }))
    }

    /// Adds to `found` what `event`'s edit of `path` concerns, `names` being the definitions whose
    /// signature it changed there, by the rules `thresholds` and `timing`.
    ///
    /// # Errors
    ///
    /// The store's errors.
    fn edit_of(
        &self,
        queries: Queries<'_>,
        (event, path, names): (&Event, &RepoPath, &[&str]),
        (thresholds, timing): (&Thresholds, &Timing),
        found: &mut Found,
    ) -> Result<(), cortex_store::StoreError> {
        let Some(edited) = queries.file_node_at(path)? else {
            return Ok(());
        };
        let links: Vec<(NodeId, Link)> = queries
            .couplings(edited, &self.nodes)?
            .into_iter()
            .filter_map(link_of)
            .collect();
        let edit = EditAt {
            session: &event.session_id,
            node: edited,
            at_ms: event.ts_ms,
        };
        let last_event = |session: &Identifier| last_event_of(session, &self.seen);
        for concern in concerned(edit, &self.held, &last_event, (&links, thresholds, timing)) {
            let Some(other) = self
                .seen
                .iter()
                .find(|seen| seen.session_id == concern.session)
            else {
                continue;
            };
            let Some((_, held_path)) = queries.node_file(concern.held)? else {
                continue;
            };
            let (other_name, editor_name) = (
                name_of(other.agent, event.agent),
                name_of(event.agent, other.agent),
            );
            let them = Party {
                agent: &other_name,
                path: held_path.as_str(),
            };
            if let Some(line) = cortex_coord::to_editor(&concern, them, path.as_str()) {
                found.lines.push((concern.is_urgent, line));
            }
            let editor = Party {
                agent: &editor_name,
                path: path.as_str(),
            };
            if let Some(notice) =
                cortex_coord::to_holder(&concern, editor, held_path.as_str(), names)
            {
                found.queued.push(NoticeRecord {
                    to_session: concern.session.clone(),
                    about: notice.subject,
                    text: notice.text,
                    at_ms: event.ts_ms,
                });
            }
        }
        Ok(())
    }
}

impl Found {
    /// What was found, the lines that act first leading and otherwise in the order found (by file,
    /// then session), one line for each happening: two sessions of one agent holding one file are
    /// said the same way, and one line says it of both, as does one held file linked to two files
    /// one edit changed.
    fn into_coordination(self) -> Coordination {
        let mut lines = self.lines;
        lines.sort_by_key(|(is_urgent, _)| !is_urgent);
        let mut told = BTreeSet::new();
        lines.retain(|(_, line)| told.insert(line.subject.clone()));
        Coordination {
            lines: lines.into_iter().map(|(_, line)| line).collect(),
            queued: self.queued,
        }
    }
}

impl Service {
    /// The coordination `event`, an edit, comes to: for each file it changed, the other live
    /// sessions holding that file or one linked to it strongly enough, outside the editing
    /// session's own family (its parent and its subagents); the line the editing session could be
    /// told of each, and the notice for each. `changed` names the definitions whose signature the
    /// edit of [`Event::edited_file`] changed, which the notices name.
    ///
    /// # Errors
    ///
    /// The store's errors.
    pub(super) fn coordinate(
        &self,
        queries: Queries<'_>,
        event: &Event,
        changed: &[String],
    ) -> Result<Coordination, cortex_store::StoreError> {
        let Some(target) = &event.tool_target else {
            return Ok(Coordination::default());
        };
        let timing = Timing::of(&self.coordination);
        let Some(around) = Around::read(queries, event, &timing)? else {
            return Ok(Coordination::default());
        };
        let thresholds = Thresholds::of(&self.coordination);
        let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
        let mut found = Found::default();
        for path in target.paths() {
            let names: &[&str] = if event.edited_file() == Some(path) {
                &changed
            } else {
                &[]
            };
            around.edit_of(
                queries,
                (event, path, names),
                (&thresholds, &timing),
                &mut found,
            )?;
        }
        Ok(found.into_coordination())
    }

    /// `answer`, the answer to `event`, with the coordination notices its session is given ahead
    /// of its own text: those waiting for it, made while an edit holds, oldest first, then the lines
    /// its own edit found, at most `coordination.max_notices` of them and `coordination.max_tokens`,
    /// each given once (`noticed` keeps what was given until the writer has it). A held-out task
    /// is given none: what waits for it waits, and what its edit found for other sessions is still
    /// queued for them. A store that cannot be read gives none, and the error is logged.
    pub(crate) fn with_notices(
        &self,
        event: &Event,
        mut answer: Answer,
        noticed: &Noticed,
    ) -> Answer {
        let Coordination { lines, queued } = std::mem::take(&mut answer.coordination);
        let timing = Timing::of(&self.coordination);
        let since_ms = event.ts_ms.saturating_sub(timing.edit_ms);
        let read = self.with_reader(|queries| {
            let (task_id, is_holdout) = task_of_answer(queries, event, self.holdout_share)?;
            // INVARIANT: silent-when-unsure (a held-out task is given nothing Cortex would say)
            if is_holdout {
                return Ok(None);
            }
            let waiting = queries.notices_waiting(&event.session_id, since_ms)?;
            let sent: Vec<NoticeSubject> = queries
                .notices_since(&event.session_id, since_ms)?
                .into_iter()
                .filter_map(|notice| notice.about)
                .collect();
            Ok(Some((task_id, waiting, sent)))
        });
        let read = read.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "a session's notices failed");
            None
        });
        let mut notices = Notices {
            queued,
            repeat_after_ms: timing.edit_ms,
            at_ms: event.ts_ms,
            ..Notices::default()
        };
        if let Some((task_id, waiting, sent)) = read {
            let giving = noticed.deciding(
                &event.session_id,
                (waiting, lines, &sent),
                (since_ms, event.ts_ms),
                |open| capped(open, &self.coordination),
            );
            let block: String = giving
                .delivered
                .iter()
                .map(|notice| notice.text.as_str())
                .chain(giving.told.iter().map(|line| line.text.as_str()))
                .map(|line| format!("{line}\n"))
                .collect();
            if !block.is_empty() {
                notices.text = Some(text_record(event, task_id, &block));
                notices.delivered = giving.delivered.iter().map(|notice| notice.id).collect();
                notices.told = giving
                    .told
                    .iter()
                    .map(|line| NoticeRecord {
                        to_session: event.session_id.clone(),
                        about: line.subject.clone(),
                        text: line.text.clone(),
                        at_ms: event.ts_ms,
                    })
                    .collect();
                answer.claims.push(Claim::Noticed(
                    event.session_id.clone(),
                    notices.delivered.clone(),
                    giving.told.into_iter().map(|line| line.subject).collect(),
                ));
                answer.text = Some(block + answer.text.as_deref().unwrap_or_default());
            }
        }
        if notices.text.is_some() || !notices.queued.is_empty() {
            answer.given.push(Given::Notices(notices));
        }
        answer
    }
}

/// Of `open`, what one answer gives under `config`: the waiting notices, oldest first, then the
/// lines, as many as `coordination.max_notices` allows and as fit in `coordination.max_tokens`,
/// counted on the text they come to; the rest wait for the session's next hook, or are found again
/// at its next edit.
fn capped(open: Giving, config: &CoordinationConfig) -> Giving {
    let most = usize::try_from(config.max_notices).unwrap_or(usize::MAX);
    let max_tokens = usize::try_from(config.max_tokens).unwrap_or(usize::MAX);
    let mut block = String::new();
    let mut fits = |text: &str| {
        let longer = format!("{block}{text}\n");
        let is_fitting = estimate_tokens(&longer) <= max_tokens;
        if is_fitting {
            block = longer;
        }
        is_fitting
    };
    // INVARIANT: budgeted (at most max_notices lines and max_tokens tokens of notices)
    let mut given = Giving::default();
    for notice in open.delivered {
        if given.delivered.len() < most && fits(&notice.text) {
            given.delivered.push(notice);
        }
    }
    for line in open.told {
        if given.delivered.len() + given.told.len() < most && fits(&line.text) {
            given.told.push(line);
        }
    }
    given
}

/// The sessions the editing session of `event` works with rather than beside: itself, the session
/// that started it and the subagents it started, as `seen` and the event say.
pub(super) fn family_of(event: &Event, seen: &[SeenSession]) -> Vec<Identifier> {
    let own = seen.iter().find(|seen| seen.session_id == event.session_id);
    let parent = event
        .parent_session_id
        .clone()
        .or_else(|| own.and_then(|own| own.parent_session_id.clone()));
    seen.iter()
        .filter(|seen| seen.parent_session_id.as_ref() == Some(&event.session_id))
        .map(|child| child.session_id.clone())
        .chain(parent)
        .chain([event.session_id.clone()])
        .collect()
}

/// When `session` was last heard from, as `seen` says; none for a session that ended after its
/// last event (an event after its end means it was taken up again).
fn last_event_of(session: &Identifier, seen: &[SeenSession]) -> Option<i64> {
    seen.iter()
        .find(|seen| seen.session_id == *session)?
        .heard_from_ms()
}

/// How a session of `agent` is named to a session of `to`: by its agent's name, or as another
/// session when both are of one agent.
fn name_of(agent: Agent, to: Agent) -> String {
    if agent == to && agent != Agent::Other {
        format!("Another {} session", agent.title())
    } else {
        agent.title().to_owned()
    }
}

/// `row` as the rules read a position.
fn rule_of(row: PositionRow) -> Position<Identifier, NodeId> {
    Position {
        session: row.record.session_id,
        node: row.record.node,
        mode: row.record.mode,
        last_ms: row.record.last_ms,
        task_ended_ms: row.task_ended_ms,
    }
}

/// `coupling` as the rules read a link; none for an edge of another type.
fn link_of(coupling: Coupling) -> Option<(NodeId, Link)> {
    let link = match coupling.edge_type {
        EdgeType::Structural => Link::Structural(coupling.weight),
        EdgeType::CoChange => Link::CoChange(coupling.weight),
        EdgeType::CoAccess | EdgeType::About | EdgeType::Containment => return None,
    };
    Some((coupling.held, link))
}

#[cfg(test)]
mod tests;
