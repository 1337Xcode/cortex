//! The handoff a session's first prompt is given: the most recent open one, left within a day by a
//! session outside its own family that is done with it (it ended, it has been quiet for
//! `coordination.handoff_idle_minutes`, or its turn stopped on an error), that the prompt concerns. A prompt concerns a handoff when one
//! of the files its retrieval weighed (the three best, known whether or not the gate opened) is a
//! file the stopped session edited or read, or when it opens by asking to go on (continue,
//! resume, pick up, carry on, keep going, finish up). A prompt that only asks to go on, naming no
//! file of any handoff, is given one only when one alone applies: with several, which it means is
//! a guess, and nothing is said. The handoff says what was left, and comes ahead of any work order. Each
//! handoff is claimed when it is given, so two first prompts answered at once do not both get it;
//! the claim is given back if its text never reaches the agent.

use cortex_coord::{FailedCheck, Handed, handoff_text};
use cortex_core::{Event, Identifier, RepoPath, SessionStatus, Ulid};
use cortex_store::{HandoffEnd, HandoffId, HandoffRow, NodeId, Queries, SeenSession, StoreError};

use super::coordination::family_of;
use crate::once::ClaimedOnce;

/// How long a handoff is offered after its session stopped: a day, in milliseconds.
const OFFERED_FOR_MS: i64 = 24 * 60 * 60 * 1_000;

/// Phrases that ask to go on with earlier work when a prompt opens with them, in any case. A bare
/// "finish" is not one: "Finish the CSV export" asks for new work as often as for old.
const GO_ON_PHRASES: [&str; 7] = [
    "continue",
    "resume",
    "pick up",
    "carry on",
    "keep going",
    "finish up",
    "finish it",
];

/// Words a prompt may open with before its go-on phrase ("please continue", "ok, carry on").
const OPENERS: [&str; 5] = ["please", "ok", "okay", "now", "so"];

/// The handoff the prompt `prompt` of `event`, starting the task `task_id`, is given, its retrieval
/// having weighed `weighed`, a session counting as done once quiet for `idle_ms` and the handoffs
/// already given claimed in `handed`: the handoff and its text, claimed. None unless this is the
/// session's first prompt and an open handoff, not given yet, applies. The prompt's own task does
/// not count: the event is queued before it is answered, so the writer may already have stored it.
///
/// # Errors
///
/// The store's errors.
pub(super) fn handoff_for(
    queries: Queries<'_>,
    (event, prompt, task_id): (&Event, &str, Ulid),
    weighed: &[NodeId],
    (idle_ms, handed): (i64, &ClaimedOnce<HandoffId>),
) -> Result<Option<(HandoffId, String)>, StoreError> {
    if queries.has_other_task(&event.session_id, task_id)? {
        return Ok(None);
    }
    let open = queries.open_handoffs_since(event.ts_ms.saturating_sub(OFFERED_FOR_MS))?;
    if open.is_empty() {
        return Ok(None);
    }
    let mut sessions: Vec<Identifier> = open
        .iter()
        .map(|row| row.record.from_session.clone())
        .chain([event.session_id.clone()])
        .collect();
    sessions.sort_unstable();
    sessions.dedup();
    let seen = queries.sessions_seen(&sessions)?;
    let family = family_of(event, &seen);
    let goes_on = asks_to_go_on(prompt);
    let mut eligible: Vec<(&HandoffRow, &SeenSession, bool)> = Vec::new();
    for row in &open {
        let record = &row.record;
        let is_concerned = concerns(weighed, &record.edited, &record.read);
        if family.contains(&record.from_session) || !(goes_on || is_concerned) {
            continue;
        }
        let Some(from) = seen
            .iter()
            .find(|seen| seen.session_id == record.from_session)
        else {
            continue;
        };
        // A conversation its user cleared set its work aside on purpose.
        if from.status == SessionStatus::Cleared
            || !is_done(from, record.end, (event.ts_ms, idle_ms))
        {
            continue;
        }
        eligible.push((row, from, is_concerned));
    }
    // INVARIANT: silent-when-unsure (a handoff only for a prompt that weighs one of its files, the
    // newest first, or that only asks to go on when one handoff alone applies; never a session's
    // own)
    let concerned: Vec<&(&HandoffRow, &SeenSession, bool)> = eligible
        .iter()
        .filter(|(_, _, is_concerned)| *is_concerned)
        .collect();
    let chosen: Vec<&(&HandoffRow, &SeenSession, bool)> = if !concerned.is_empty() {
        concerned
    } else if eligible.len() == 1 {
        eligible.iter().collect()
    } else {
        Vec::new()
    };
    for (row, from, _) in chosen {
        let agent = from.agent.title();
        if let Some(text) = text_of(queries, row, (agent, event.ts_ms))?
            && handed.claim(row.id)
        {
            return Ok(Some((row.id, text)));
        }
    }
    Ok(None)
}

/// The text of `row`, left by a session of `agent`, as of `now_ms`: its files by the paths the
/// store holds now (a file gone is left out). None when nothing is left to say.
fn text_of(
    queries: Queries<'_>,
    row: &HandoffRow,
    (agent, now_ms): (&str, i64),
) -> Result<Option<String>, StoreError> {
    let paths = |nodes: &[NodeId]| -> Result<Vec<RepoPath>, StoreError> {
        let mut paths = Vec::new();
        for node in nodes {
            if let Some((_, path)) = queries.node_file(*node)? {
                paths.push(path);
            }
        }
        Ok(paths)
    };
    let (edited, read) = (paths(&row.record.edited)?, paths(&row.record.read)?);
    let edited: Vec<&str> = edited.iter().map(RepoPath::as_str).collect();
    let read: Vec<&str> = read.iter().map(RepoPath::as_str).collect();
    let failure = row.record.last_failure.as_ref().map(|failure| FailedCheck {
        runner: failure.runner.as_str(),
        kind: failure.kind.name(),
        exit_code: failure.exit_code,
        counts: failure.counts,
    });
    Ok(handoff_text(&Handed {
        agent,
        ago_ms: now_ms.saturating_sub(row.record.created_at_ms),
        is_cut_off: row.record.end != HandoffEnd::Answered,
        edited: &edited,
        read: &read,
        failure,
        open_warnings: row.record.open_warnings.len(),
    }))
}

/// Whether the session `from` is done with the work its handoff names, that handoff's task having
/// ended as `end`, as of `now_ms`: its turn stopped on an error (the user is likely to switch
/// agents at once), it ended, or it has been quiet for `idle_ms`. A session still at work keeps
/// its handoff.
fn is_done(from: &SeenSession, end: HandoffEnd, (now_ms, idle_ms): (i64, i64)) -> bool {
    end == HandoffEnd::Stopped
        || from.ended_ms.is_some()
        || from
            .last_event_ms
            .is_some_and(|last_ms| now_ms.saturating_sub(last_ms) >= idle_ms)
}

/// Whether a prompt whose retrieval weighed `weighed` concerns a handoff naming `edited` and `read`:
/// one of the files weighed is one of them.
fn concerns<T: PartialEq>(weighed: &[T], edited: &[T], read: &[T]) -> bool {
    weighed
        .iter()
        .any(|node| edited.contains(node) || read.contains(node))
}

/// Whether `prompt` opens by asking to go on with earlier work: after any of [`OPENERS`], one of
/// [`GO_ON_PHRASES`] as whole words. A phrase later in the prompt ("make the parser continue
/// after a bad line") describes the work, and does not ask for an earlier session's.
fn asks_to_go_on(prompt: &str) -> bool {
    let words: Vec<String> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let opened = words
        .iter()
        .position(|word| !OPENERS.contains(&word.as_str()))
        .and_then(|at| words.get(at..))
        .unwrap_or_default();
    GO_ON_PHRASES.iter().any(|phrase| {
        let phrase: Vec<&str> = phrase.split(' ').collect();
        opened.len() >= phrase.len() && opened.iter().zip(&phrase).all(|(word, part)| word == part)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test's result: any error fails it.
    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A session `s1` of Claude Code, ended at `ended_ms` and last heard from at `last_ms`.
    fn seen(ended_ms: Option<i64>, last_ms: Option<i64>) -> TestResult<SeenSession> {
        Ok(SeenSession {
            session_id: Identifier::new("s1")?,
            agent: cortex_core::Agent::Claude,
            parent_session_id: None,
            ended_ms,
            last_event_ms: last_ms,
            status: SessionStatus::Live,
        })
    }

    /// At 100 minutes, with 30 minutes quiet making a session done: a turn stopped on an error is
    /// done at once; an answered or left task's session once it ended, or once its last event is
    /// 30 minutes old or more (70 is, 71 is not), never while it has no event and has not ended.
    #[test]
    fn a_session_is_done_with_its_handoff_once_it_ends_or_is_quiet() -> TestResult {
        let minute = 60_000;
        let at = (100 * minute, 30 * minute);
        assert!(is_done(
            &seen(None, Some(100 * minute))?,
            HandoffEnd::Stopped,
            at
        ));
        assert!(is_done(
            &seen(Some(99 * minute), Some(99 * minute))?,
            HandoffEnd::Answered,
            at
        ));
        assert!(is_done(
            &seen(None, Some(70 * minute))?,
            HandoffEnd::Left,
            at
        ));
        assert!(!is_done(
            &seen(None, Some(71 * minute))?,
            HandoffEnd::Answered,
            at
        ));
        assert!(!is_done(&seen(None, None)?, HandoffEnd::Left, at));
        Ok(())
    }

    /// A handoff that edited 1 and read 2: a prompt weighing 2 or 1 concerns it, one weighing only
    /// 3 and 4, or nothing, does not.
    #[test]
    fn a_prompt_concerns_a_handoff_by_a_file_it_weighed() {
        let (edited, read) = ([1], [2]);
        assert!(concerns(&[4, 2], &edited, &read));
        assert!(concerns(&[1], &edited, &read));
        assert!(!concerns(&[3, 4], &edited, &read));
        assert!(!concerns(&[], &edited, &read));
    }

    /// A prompt asks to go on when it opens with a go-on phrase, after any opener, in any case and
    /// across punctuation; a phrase later in the prompt describes the work, a bare "finish" asks for
    /// new work, and a word that only holds a phrase does not count.
    #[test]
    fn a_prompt_asks_to_go_on_when_it_opens_with_it() {
        for prompt in [
            "Continue.",
            "please RESUME the change",
            "Ok, carry on with divide",
            "pick up where it stopped",
            "Keep going",
            "finish up the refactor",
        ] {
            assert!(asks_to_go_on(prompt), "{prompt}");
        }
        for prompt in [
            "Make the parser continue after a bad line",
            "We should continue to support Python 3.8",
            "Finish the CSV export",
            "continued fraction",
            "the finished parts",
            "pick the upper one",
            "please",
            "",
        ] {
            assert!(!asks_to_go_on(prompt), "{prompt}");
        }
    }

    /// A Codex session `session`'s first prompt "Continue." at 2,000 ms.
    fn first_prompt(session: &str) -> TestResult<(cortex_core::Event, Ulid)> {
        let ts_ms = 2_000;
        let event = cortex_core::Event {
            event_id: Ulid::from_seed(ts_ms, session.as_bytes()),
            ts_ms: 2_000,
            agent: cortex_core::Agent::Codex,
            agent_version: None,
            session_id: Identifier::new(session)?,
            parent_session_id: None,
            cwd: std::path::PathBuf::from("/repo"),
            repo_root: std::path::PathBuf::from("/repo"),
            hook: cortex_core::HookKind::Prompt,
            source: None,
            tool: None,
            tool_target: None,
            tool_result: None,
            prompt_text: Some("Continue.".to_owned()),
            transcript_path: None,
            model: None,
            subagent_type: None,
            compact_trigger: None,
            end_reason: None,
            is_probe: false,
        };
        let task_id = cortex_core::task_id_of_prompt(event.event_id, event.ts_ms);
        Ok((event, task_id))
    }

    /// One open handoff, left by a Claude Code session `s1` that ended, read by the first prompts
    /// of two Codex sessions answered before either is recorded: the first is given it and claims
    /// it, the second is given nothing; once the first's text is known not to have reached its
    /// agent, the claim given back, the second is given it.
    #[test]
    fn a_handoff_is_given_to_one_first_prompt() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut store = crate::test_store::writer(root.path())?;
        let ops = crate::test_store::add_file(&mut store, "calc/ops.py", 1)?;
        let session = |id: &str, agent| -> TestResult<cortex_store::SessionRecord> {
            Ok(cortex_store::SessionRecord {
                session_id: Identifier::new(id)?,
                agent,
                agent_version: None,
                model: None,
                parent_session_id: None,
                cwd: None,
                transcript_path: None,
                started_at_ms: 100,
            })
        };
        let mut transaction = store.transaction()?;
        transaction.record_session(&session("s1", cortex_core::Agent::Claude)?)?;
        transaction.record_session(&session("s2", cortex_core::Agent::Codex)?)?;
        transaction.record_session(&session("s3", cortex_core::Agent::Codex)?)?;
        transaction.end_session(&Identifier::new("s1")?, (1_500, SessionStatus::Ended))?;
        transaction.leave_handoff(&cortex_store::HandoffRecord {
            from_session: Identifier::new("s1")?,
            task_id: Ulid::from_seed(1_000, b"s1"),
            end: HandoffEnd::Left,
            edited: vec![ops],
            read: Vec::new(),
            open_warnings: Vec::new(),
            last_failure: None,
            created_at_ms: 1_500,
        })?;
        transaction.commit()?;
        let handed = ClaimedOnce::default();
        let idle_ms = 30 * 60 * 1_000;
        let given = |session: &str| -> TestResult<Option<HandoffId>> {
            let (event, task_id) = first_prompt(session)?;
            let found = handoff_for(
                store.queries(),
                (&event, "Continue.", task_id),
                &[],
                (idle_ms, &handed),
            )?;
            Ok(found.map(|(handoff, _)| handoff))
        };
        let first = given("s2")?.ok_or("s2 is given the handoff")?;
        assert_eq!(given("s3")?, None);
        handed.release(&first);
        assert_eq!(given("s3")?, Some(first));
        Ok(())
    }

    /// Two open handoffs, `s1`'s on `calc/ops.py` and `s4`'s on `main.py`, both sessions ended:
    /// "Continue." alone could mean either, and is given neither; a prompt whose retrieval weighed
    /// `main.py` is given `s4`'s.
    #[test]
    fn a_bare_go_on_with_two_handoffs_is_given_neither() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut store = crate::test_store::writer(root.path())?;
        let ops = crate::test_store::add_file(&mut store, "calc/ops.py", 1)?;
        let main = crate::test_store::add_file(&mut store, "main.py", 1)?;
        let mut transaction = store.transaction()?;
        let mut left = Vec::new();
        for (session, file, at_ms) in [("s1", ops, 1_500), ("s4", main, 1_600)] {
            transaction.record_session(&cortex_store::SessionRecord {
                session_id: Identifier::new(session)?,
                agent: cortex_core::Agent::Claude,
                agent_version: None,
                model: None,
                parent_session_id: None,
                cwd: None,
                transcript_path: None,
                started_at_ms: 100,
            })?;
            transaction.end_session(&Identifier::new(session)?, (at_ms, SessionStatus::Ended))?;
            left.push(transaction.leave_handoff(&cortex_store::HandoffRecord {
                from_session: Identifier::new(session)?,
                task_id: Ulid::from_seed(u64::try_from(at_ms)?, session.as_bytes()),
                end: HandoffEnd::Left,
                edited: vec![file],
                read: Vec::new(),
                open_warnings: Vec::new(),
                last_failure: None,
                created_at_ms: at_ms,
            })?);
        }
        transaction.commit()?;
        let handed = ClaimedOnce::default();
        let (event, task_id) = first_prompt("s2")?;
        let given = |weighed: &[NodeId]| -> TestResult<Option<HandoffId>> {
            let found = handoff_for(
                store.queries(),
                (&event, "Continue.", task_id),
                weighed,
                (30 * 60 * 1_000, &handed),
            )?;
            Ok(found.map(|(handoff, _)| handoff))
        };
        assert_eq!(given(&[])?, None);
        assert_eq!(given(&[main])?, left.get(1).copied());
        Ok(())
    }
}
