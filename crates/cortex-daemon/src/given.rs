//! What Cortex gave an agent, or would have given, as the writer records it after the event it
//! answered: a primer's or a prompt's injection, the text of a recap, a read's hint or a stop's
//! reason, or the warnings an edit got with their text. Every text is an injection, so that what
//! Cortex added to an agent's context is counted whole. What reached the agent is recorded as
//! given; an injection that did not is recorded as not delivered, for the holdout and the labels,
//! and warnings that did not are not recorded at all, since nobody was warned.

use cortex_core::{ContentHash, Event, Ulid, estimate_tokens};
use cortex_store::{InjectionRecord, WarningRecord};

/// What one answer gave, or would have given.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Given {
    /// A primer or a prompt's retrieval.
    Injection(InjectionRecord),
    /// The warnings an edit got: one row for each file each of them named, and their text.
    Warnings(Vec<WarningRecord>, InjectionRecord),
}

impl Given {
    /// The line kept among the latest errors when this gives way to newer records waiting.
    pub(crate) fn dropped_message(&self) -> &'static str {
        match self {
            Self::Injection(_) => "an injection was not recorded: too many waited",
            Self::Warnings(..) => "an edit's warnings were not recorded: too many waited",
        }
    }
}

/// The record of `text`, given at the hook of `event` in task `task_id` (none outside a task):
/// its hash and tokens, no files and no features, neither kept back nor held out. Only a prompt's
/// work order is labelled at its task's end, so a text recorded so counts in what was given and
/// never in the work orders' precision.
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
/// not at all. A reply without text gave nothing either way: an injection's record (kept back, or
/// held out with what it would have given) stands, and warnings without text are none.
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
}
