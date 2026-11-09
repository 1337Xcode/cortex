//! What Cortex gave an agent, or would have given, as the writer records it after the event it
//! answered: a primer's or a prompt's injection, the text of a recap, a read's hint or a stop's
//! reason, the warnings an edit got with their text, or the coordination notices an answer gave,
//! told or queued for other sessions. Every text is an injection, so that what Cortex added to an
//! agent's context is counted whole. What reached the agent is recorded as given; an injection
//! that did not is recorded as not delivered, for the holdout and the labels, and warnings and
//! notices that did not are not recorded as given at all, since nobody was told: the notices wait
//! again. The notices an edit queued for other sessions are queued however its own answer fared.

use cortex_core::{ContentHash, Event, Identifier, Ulid, estimate_tokens};
use cortex_store::{HandoffId, InjectionRecord, NoticeId, NoticeRecord, WarningRecord};

/// What one answer gave, or would have given.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Given {
    /// A primer or a prompt's retrieval.
    Injection(InjectionRecord),
    /// The warnings an edit got: one row for each file each of them named, and their text.
    Warnings(Vec<WarningRecord>, InjectionRecord),
    /// What an answer did with coordination notices.
    Notices(Notices),
    /// A handoff given at a session's first prompt: which handoff, the session given it, and the
    /// record of its text. Recorded only once the text reached the agent, which closes that
    /// handoff and no other.
    Handoff(HandoffId, Identifier, InjectionRecord),
}

/// What one answer did with coordination notices: those it gave and those it queued.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Notices {
    /// The waiting notices it gave, marked delivered at `at_ms`.
    pub(crate) delivered: Vec<NoticeId>,
    /// The lines it told its own session, recorded as delivered as they were made.
    pub(crate) told: Vec<NoticeRecord>,
    /// The notices an edit queued for the other sessions it concerns.
    pub(crate) queued: Vec<NoticeRecord>,
    /// How long a notice is not queued again for a session that was sent or given the same text,
    /// in milliseconds.
    pub(crate) repeat_after_ms: i64,
    /// When the answer gave them, in UTC milliseconds.
    pub(crate) at_ms: i64,
    /// The record of the text the notices given came to; none when it gave none.
    pub(crate) text: Option<InjectionRecord>,
}

impl Given {
    /// The line kept among the latest errors when this gives way to newer records waiting.
    pub(crate) fn dropped_message(&self) -> &'static str {
        match self {
            Self::Injection(_) => "an injection was not recorded: too many waited",
            Self::Warnings(..) => "an edit's warnings were not recorded: too many waited",
            Self::Notices(_) => {
                "an answer's coordination notices were not recorded: too many waited"
            }
            Self::Handoff(..) => "a handoff given was not recorded: too many waited",
        }
    }
}

/// The record of `text`, given at the hook of `event` in task `task_id` (none outside a task):
/// its hash and tokens, no files and no features, neither kept back nor held out. Only a prompt's
/// work order, which alone carries the gate's features, is labelled at its task's end or read back
/// as a work order, so a text recorded so, the notices a prompt is given included, counts in what
/// was given and never in the work orders' precision.
pub(crate) fn text_record(event: &Event, task_id: Option<Ulid>, text: &str) -> InjectionRecord {
    InjectionRecord {
        task_id,
        session_id: event.session_id.clone(),
        hook: event.hook,
        at_ms: event.ts_ms,
        files: Vec::new(),
        tokens: u32::try_from(estimate_tokens(text)).unwrap_or(u32::MAX),
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(text.as_bytes())),
        features: None,
    }
}

/// `injection` as recorded when its answer could not be written: what would have been given, but
/// no text that reached the agent, so no hash and no tokens.
pub(crate) fn not_delivered(injection: InjectionRecord) -> InjectionRecord {
    InjectionRecord {
        tokens: 0,
        text_hash: None,
        ..injection
    }
}

/// What is recorded of `given` once its reply was written, or could not be (`is_written`): a reply
/// holding text (`holds_text`) that could not be written, or was written after its hook stopped
/// reading (`is_late`), gave the agent nothing: an injection is recorded as not delivered, warnings
/// not at all, notices as queued only. A reply without text gave nothing either way: an
/// injection's record (kept back, or held out with what it would have given) stands, and warnings
/// without text are none.
pub(crate) fn recorded(
    given: Given,
    holds_text: bool,
    is_written: bool,
    is_late: bool,
) -> Option<Given> {
    let reached = holds_text && is_written && !is_late;
    match given {
        Given::Injection(injection) if holds_text && !reached => {
            Some(Given::Injection(not_delivered(injection)))
        }
        Given::Injection(injection) => Some(Given::Injection(injection)),
        Given::Warnings(rows, text) if reached && !rows.is_empty() => {
            Some(Given::Warnings(rows, text))
        }
        Given::Warnings(..) => None,
        // A handoff that did not reach the agent stays open for the next session.
        Given::Handoff(handoff, by, text) if reached => Some(Given::Handoff(handoff, by, text)),
        Given::Handoff(..) => None,
        Given::Notices(notices) if reached => Some(Given::Notices(notices)),
        // Nothing given reached the agent: what was given waits again, and only what was queued
        // for the other sessions stands.
        Given::Notices(notices) if notices.queued.is_empty() => None,
        Given::Notices(notices) => Some(Given::Notices(Notices {
            delivered: Vec::new(),
            told: Vec::new(),
            text: None,
            ..notices
        })),
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::{ContentHash, HookKind, Identifier, Ulid, WarningKind};
    use cortex_store::{InjectionRecord, WarningRecord};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// A prompt's injection that gave a work order of 120 tokens.
    fn injection() -> TestResult<InjectionRecord> {
        Ok(InjectionRecord {
            task_id: None,
            session_id: Identifier::new("s1")?,
            hook: HookKind::Prompt,
            at_ms: 5,
            files: Vec::new(),
            tokens: 120,
            confidence: None,
            is_gated_out: false,
            is_holdout: false,
            text_hash: Some(ContentHash::of(b"a work order")),
            features: None,
        })
    }

    /// An injection that could not be written is recorded as given to no one: what the retrieval
    /// gave stays, its files and its features, but there is no text's hash and no token.
    #[test]
    fn an_injection_not_written_is_not_delivered() -> TestResult {
        let given = injection()?;
        let lost = not_delivered(given.clone());
        assert_eq!((lost.tokens, lost.text_hash), (0, None));
        assert_eq!(
            InjectionRecord {
                tokens: given.tokens,
                text_hash: given.text_hash,
                ..lost
            },
            given
        );
        Ok(())
    }

    /// A reply holding text is recorded as given only when it was written before its hook stopped
    /// reading. An injection written late, or not written, is recorded as not delivered, and one
    /// without text, a held-out task's, keeps what it would have given whatever came of its
    /// writing. Warnings written late, not written, or without text are not recorded at all.
    #[test]
    fn a_reply_holding_text_is_given_only_when_written_in_time() -> TestResult {
        let given = injection()?;
        let lost = Some(Given::Injection(not_delivered(given.clone())));
        let as_given = || Given::Injection(given.clone());
        assert_eq!(recorded(as_given(), true, true, false), Some(as_given()));
        assert_eq!(recorded(as_given(), true, true, true), lost);
        assert_eq!(recorded(as_given(), true, false, false), lost);
        let held_out = Given::Injection(InjectionRecord {
            is_holdout: true,
            ..given.clone()
        });
        assert_eq!(
            recorded(held_out.clone(), false, false, true),
            Some(held_out)
        );

        let root = tempfile::tempdir()?;
        let mut store = writer(root.path())?;
        let (source, target) = (
            add_file(&mut store, "a.py", 1)?,
            add_file(&mut store, "b.py", 1)?,
        );
        let warning = WarningRecord {
            task_id: Ulid::from_seed(1, b"task"),
            at_ms: 5,
            kind: WarningKind::CoChange,
            source,
            target,
            strength: 0.75,
        };
        let warned = || Given::Warnings(vec![warning], given.clone());
        assert_eq!(recorded(warned(), true, true, false), Some(warned()));
        assert_eq!(recorded(warned(), true, true, true), None);
        assert_eq!(recorded(warned(), true, false, false), None);
        assert_eq!(recorded(warned(), false, true, false), None);
        assert_eq!(
            recorded(
                Given::Warnings(Vec::new(), given.clone()),
                true,
                true,
                false
            ),
            None
        );
        Ok(())
    }
    /// Notices that reached the agent are recorded whole. When the reply was late, not written or
    /// held no text, what it gave waits again: only the notices queued for the other sessions are
    /// recorded, and nothing at all when it queued none.
    #[test]
    fn notices_given_are_recorded_only_when_they_reached_the_agent() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut store = writer(root.path())?;
        let delivered = {
            let mut transaction = store.transaction()?;
            let notice = NoticeRecord {
                to_session: Identifier::new("s1")?,
                text: "waiting".to_owned(),
                at_ms: 1,
            };
            let id = transaction.queue_notice(&notice, 0)?.ok_or("queued")?;
            transaction.commit()?;
            id
        };
        let line = |to: &str, text: &str| -> TestResult<NoticeRecord> {
            Ok(NoticeRecord {
                to_session: Identifier::new(to)?,
                text: text.to_owned(),
                at_ms: 5,
            })
        };
        let given = Notices {
            delivered: vec![delivered],
            told: vec![line("s1", "told")?],
            queued: vec![line("s2", "queued")?],
            repeat_after_ms: 1_800_000,
            at_ms: 5,
            text: Some(injection()?),
        };
        let whole = || Given::Notices(given.clone());
        let queued_only = Some(Given::Notices(Notices {
            delivered: Vec::new(),
            told: Vec::new(),
            text: None,
            ..given.clone()
        }));
        assert_eq!(recorded(whole(), true, true, false), Some(whole()));
        assert_eq!(recorded(whole(), true, true, true), queued_only);
        assert_eq!(recorded(whole(), true, false, false), queued_only);
        assert_eq!(recorded(whole(), false, true, false), queued_only);
        let nothing_queued = Given::Notices(Notices {
            queued: Vec::new(),
            ..given.clone()
        });
        assert_eq!(recorded(nothing_queued, true, false, false), None);
        Ok(())
    }
}
