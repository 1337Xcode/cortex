//! The rules around an edit's coordination that need no store: whose sessions count as the editing
//! session's own, which sessions are live, how an agent is named, and how much one answer gives.

use std::path::PathBuf;

use cortex_core::{Agent, CoordinationConfig, Event, HookKind, Identifier, Ulid};
use cortex_store::{NoticeRecord, NoticeRow, Reader, SeenSession};

use super::{capped, family_of, last_event_of, name_of};
use crate::noticed::Giving;
use crate::test_store::{TestResult, writer};

/// What the store knows of `session` of `agent`, started by `parent`, ended at `ended_ms` and last
/// heard from at `last_ms`.
fn seen(
    session: &str,
    parent: Option<&str>,
    (ended_ms, last_ms): (Option<i64>, Option<i64>),
) -> TestResult<SeenSession> {
    Ok(SeenSession {
        session_id: Identifier::new(session)?,
        agent: Agent::Claude,
        parent_session_id: parent.map(Identifier::new).transpose()?,
        ended_ms,
        last_event_ms: last_ms,
    })
}

/// An edit's event in `session`, started by `parent` as the payload says.
fn edit_in(session: &str, parent: Option<&str>) -> TestResult<Event> {
    Ok(Event {
        event_id: Ulid::from_seed(1, b"edit"),
        ts_ms: 1_000,
        agent: Agent::Claude,
        agent_version: None,
        session_id: Identifier::new(session)?,
        parent_session_id: parent.map(Identifier::new).transpose()?,
        cwd: PathBuf::from("/repo"),
        repo_root: PathBuf::from("/repo"),
        hook: HookKind::PostTool,
        source: None,
        tool: None,
        tool_target: None,
        tool_result: None,
        prompt_text: None,
        transcript_path: None,
        model: None,
        subagent_type: None,
        compact_trigger: None,
        end_reason: None,
        is_probe: false,
    })
}

/// Worked by hand: `main` started `sub1` and `sub2`; `other` is a session of its own, and `own`
/// a subagent of `other`. An edit in `main` counts `main`, `sub1` and `sub2` as its own; one in
/// `sub1` counts `sub1` and `main` (its parent, from the store or the payload), not its sibling
/// `sub2`, which works beside it; one in `other` counts `other` and `own`.
#[test]
fn an_edit_s_own_family_is_its_parent_and_its_subagents() -> TestResult {
    let sessions = [
        seen("main", None, (None, Some(1)))?,
        seen("sub1", Some("main"), (None, Some(1)))?,
        seen("sub2", Some("main"), (None, Some(1)))?,
        seen("other", None, (None, Some(1)))?,
        seen("own", Some("other"), (None, Some(1)))?,
    ];
    let family = |event: &Event| -> Vec<String> {
        let mut family: Vec<String> = family_of(event, &sessions)
            .iter()
            .map(|session| session.as_str().to_owned())
            .collect();
        family.sort_unstable();
        family
    };
    assert_eq!(family(&edit_in("main", None)?), ["main", "sub1", "sub2"]);
    assert_eq!(family(&edit_in("sub1", None)?), ["main", "sub1"]);
    assert_eq!(family(&edit_in("sub1", Some("main"))?), ["main", "sub1"]);
    assert_eq!(family(&edit_in("other", None)?), ["other", "own"]);
    Ok(())
}

/// A session is heard from at its last event, unless it ended at or after it: ended at 50 after
/// an event at 40, it is not live; an event at 60 after an end at 50 means it was taken up again;
/// one with no event, or not stored, has none.
#[test]
fn a_session_that_ended_is_not_heard_from() -> TestResult {
    let sessions = [
        seen("open", None, (None, Some(40)))?,
        seen("ended", None, (Some(50), Some(40)))?,
        seen("ended_at_last", None, (Some(40), Some(40)))?,
        seen("resumed", None, (Some(50), Some(60)))?,
        seen("silent", None, (None, None))?,
    ];
    let last = |session: &str| -> TestResult<Option<i64>> {
        Ok(last_event_of(&Identifier::new(session)?, &sessions))
    };
    assert_eq!(last("open")?, Some(40));
    assert_eq!(last("ended")?, None);
    assert_eq!(last("ended_at_last")?, None);
    assert_eq!(last("resumed")?, Some(60));
    assert_eq!(last("silent")?, None);
    assert_eq!(last("unknown")?, None);
    Ok(())
}

/// Another agent is named by its name; a session of the same agent as another session of it; an
/// unknown agent as another agent either way.
#[test]
fn an_agent_is_named_by_its_own_name_or_as_another_session() {
    assert_eq!(name_of(Agent::Codex, Agent::Claude), "Codex");
    assert_eq!(name_of(Agent::Claude, Agent::Codex), "Claude Code");
    assert_eq!(
        name_of(Agent::Claude, Agent::Claude),
        "Another Claude Code session"
    );
    assert_eq!(name_of(Agent::Other, Agent::Other), "Another agent");
}

/// Notices waiting for one session, saying `texts`, as the store gives them back: only the store
/// makes their rows. The folder holding the store goes with them.
fn waiting(texts: &[&str]) -> TestResult<(tempfile::TempDir, Vec<NoticeRow>)> {
    let root = tempfile::tempdir()?;
    let mut store = writer(root.path())?;
    let session = Identifier::new("s1")?;
    let mut transaction = store.transaction()?;
    for (text, at_ms) in texts.iter().zip(1..) {
        let notice = NoticeRecord {
            to_session: session.clone(),
            text: (*text).to_owned(),
            at_ms,
        };
        transaction.queue_notice(&notice, 0)?;
    }
    transaction.commit()?;
    let reader = Reader::open(&root.path().join(".cortex"), 0)?;
    let rows = reader.queries().notices_waiting(&session, 0)?;
    Ok((root, rows))
}

/// Worked by hand. Of two waiting notices and two lines, at most 3 are given: both waiting ones,
/// oldest first, then the first line. With 0, none. A cap of 8 tokens (32 bytes at 4 a token, each
/// line ending in a line break) fits a notice of 20 characters and one of 10 (21 + 11 = 32 bytes),
/// and no line after them: one of 10 would make 43 bytes, one of 2 35.
#[test]
fn an_answer_gives_at_most_its_notices_and_its_tokens() -> TestResult {
    let (_short_store, short) = waiting(&["first", "second"])?;
    let open = || Giving {
        delivered: short.clone(),
        told: vec!["told one".to_owned(), "told two".to_owned()],
    };
    let config = |max_notices, max_tokens| CoordinationConfig {
        max_notices,
        max_tokens,
        ..CoordinationConfig::default()
    };
    assert_eq!(
        capped(open(), &config(3, 200)),
        Giving {
            delivered: short.clone(),
            told: vec!["told one".to_owned()],
        }
    );
    assert_eq!(capped(open(), &config(0, 200)), Giving::default());
    let (long, ten) = ("a".repeat(20), "b".repeat(10));
    let (_long_store, rows) = waiting(&[&long, &ten])?;
    let tight = Giving {
        delivered: rows.clone(),
        told: vec!["c".repeat(10), "d".repeat(2)],
    };
    assert_eq!(
        capped(tight, &config(10, 8)),
        Giving {
            delivered: rows,
            told: Vec::new(),
        }
    );
    Ok(())
}
