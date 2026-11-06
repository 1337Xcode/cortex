//! What Cortex's texts to a session came to: for each hook they answered, how many reached the
//! agent and the tokens they took as Cortex estimated them; and the most they can have added to
//! the input the sessions report.
//!
//! A text stays in the agent's context from the turn it arrives, so it is read again with every
//! later turn: a text of `n` tokens adds at most `n` times the session's turns to its input. A text
//! that arrives after the first turn adds less, so the share is an upper bound.

use std::collections::BTreeMap;

use cortex_store::InjectionRecord;
use serde::{Deserialize, Serialize};

/// How many texts of one kind reached the agent, and their tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sent {
    /// The texts.
    pub(crate) count: u64,
    /// Their tokens, as Cortex estimated them.
    pub(crate) tokens: u64,
}

impl Sent {
    /// The sum of `self` and `other`.
    #[must_use]
    pub(crate) fn add(self, other: Self) -> Self {
        Self {
            count: self.count + other.count,
            tokens: self.tokens + other.tokens,
        }
    }
}

/// The texts that reached a session's agent, by the name of the hook they answered.
pub(crate) type Texts = BTreeMap<String, Sent>;

/// The texts of `records` that reached the agent, by the hook they answered.
pub(crate) fn texts_of<'a>(records: impl IntoIterator<Item = &'a InjectionRecord>) -> Texts {
    let mut texts = Texts::new();
    for record in records {
        if !record.was_delivered() {
            continue;
        }
        let sent = texts.entry(record.hook.name().to_owned()).or_default();
        *sent = sent.add(Sent {
            count: 1,
            tokens: u64::from(record.tokens),
        });
    }
    texts
}

/// `texts` and `more` added together, kind by kind.
pub(crate) fn add_texts(mut texts: Texts, more: &Texts) -> Texts {
    for (kind, sent) in more {
        let held = texts.entry(kind.clone()).or_default();
        *held = held.add(*sent);
    }
    texts
}

/// The most the texts can have added to the sessions' input: over `sessions`, each given as its
/// texts, its turns and its cumulative input, the sum of text tokens times turns against the sum of
/// input. None when the sessions report no input.
#[must_use]
pub(crate) fn share_of_input(sessions: &[(&Texts, u64, u64)]) -> Option<f64> {
    let (read, input) = sessions.iter().fold(
        (0_u64, 0_u64),
        |(read, input), (texts, turns, cumulative)| {
            let tokens: u64 = texts.values().map(|sent| sent.tokens).sum();
            (
                read.saturating_add(tokens.saturating_mul(*turns)),
                input.saturating_add(*cumulative),
            )
        },
    );
    #[expect(clippy::cast_precision_loss, reason = "token counts stay below 2^52")]
    (input > 0).then(|| read as f64 / input as f64)
}

#[cfg(test)]
mod tests {
    use cortex_core::{ContentHash, HookKind, Identifier};

    use super::*;

    /// A text of `tokens` answering `hook`, delivered or kept back by the gate.
    fn text(hook: HookKind, tokens: u32, is_delivered: bool) -> anyhow::Result<InjectionRecord> {
        Ok(InjectionRecord {
            task_id: None,
            session_id: Identifier::new("s")?,
            hook,
            at_ms: 1,
            files: Vec::new(),
            tokens,
            confidence: None,
            is_gated_out: !is_delivered,
            is_holdout: false,
            text_hash: Some(ContentHash::of(b"text")),
            features: None,
        })
    }

    /// Only the texts that reached the agent count, by hook: a primer of 40 tokens, two warnings
    /// of 30 and 50, and a work order of 200 the gate kept back give the session start 1 text of
    /// 40 tokens and the edits 2 of 80; added to another session's work order of 300 tokens, the
    /// prompt kind holds 1 of 300.
    #[test]
    fn texts_are_counted_by_kind() -> anyhow::Result<()> {
        let records = [
            text(HookKind::SessionStart, 40, true)?,
            text(HookKind::PostTool, 30, true)?,
            text(HookKind::PostTool, 50, true)?,
            text(HookKind::Prompt, 200, false)?,
        ];
        let texts = texts_of(&records);
        let sent = |count, tokens| Sent { count, tokens };
        assert_eq!(
            texts,
            Texts::from([
                ("post_tool".to_owned(), sent(2, 80)),
                ("session_start".to_owned(), sent(1, 40)),
            ])
        );
        let other = texts_of(&[text(HookKind::Prompt, 300, true)?]);
        let both = add_texts(texts, &other);
        assert_eq!(both.get("prompt"), Some(&sent(1, 300)));
        assert_eq!(both.len(), 3);
        Ok(())
    }

    /// Worked by hand: one session's 120 tokens over 10 turns are read 1,200 times, another's 50
    /// over 4 turns 200 times, 1,400 against 100,000 + 40,000 of input: 0.01. Sessions with no
    /// input give no share.
    #[test]
    fn the_share_is_tokens_times_turns_over_input() {
        let one = Texts::from([(
            "prompt".to_owned(),
            Sent {
                count: 1,
                tokens: 120,
            },
        )]);
        let two = Texts::from([(
            "session_start".to_owned(),
            Sent {
                count: 1,
                tokens: 50,
            },
        )]);
        let share = share_of_input(&[(&one, 10, 100_000), (&two, 4, 40_000)]);
        assert!(
            share.is_some_and(|share| (share - 0.01).abs() < 1e-12),
            "{share:?}"
        );
        assert_eq!(share_of_input(&[(&one, 10, 0)]), None);
        assert_eq!(share_of_input(&[]), None);
    }
}
