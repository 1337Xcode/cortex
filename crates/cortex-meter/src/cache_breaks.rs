//! Where a session lost its prompt cache, read from the responses its transcripts report, and the
//! cause that most likely lost it.
//!
//! A break is a response that wrote most of its cached input anew while its cache reads fell to
//! under half of what the response before it read: the cache the conversation had built was lost.
//! Each conversation is looked at on its own, the session's and each subagent's, because each
//! keeps a cache of its own. A break is given the first cause that holds of a model switch, a
//! compaction, an idle gap longer than the cache lives and text Cortex gave, or none when nothing
//! known explains it. The cache lives five minutes unless every cache write of the conversation
//! before the break was for an hour, as the transcript reports the split; then it lives an hour.
//!
//! The responses are the meter's rows, already one per message: a transcript line written again
//! for each content block of a response is counted once, with the largest counts any copy
//! reported, so nothing here counts a response twice.

use std::collections::BTreeMap;

use cortex_core::Identifier;
use cortex_store::UsageRow;
use serde::Serialize;

/// How long the provider keeps a cache entry written for five minutes that is not read again, in
/// milliseconds. A response written longer than that after the one before it may have found its
/// cache expired.
pub(crate) const FIVE_MINUTE_LIFETIME_MS: i64 = 5 * 60 * 1000;

/// How long the provider keeps a cache entry written for an hour that is not read again, in
/// milliseconds.
pub(crate) const ONE_HOUR_LIFETIME_MS: i64 = 60 * 60 * 1000;

/// Thousandths in a whole, the unit of the two shares below.
const PER_MILLE: u128 = 1_000;

/// A response wrote its cached input anew when its cache writes are more than this share, in
/// thousandths, of its cached input (cache writes and cache reads together): more than half, where
/// the cache was mostly rebuilt rather than extended.
const WRITE_SHARE_BREAK_PER_MILLE: u128 = 500;

/// A response lost the cache when its cache reads fell below this share, in thousandths, of the
/// previous response's: under half, where most of what was cached was not found.
const READ_KEPT_BREAK_PER_MILLE: u128 = 500;

/// Why a cache break most likely happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakCause {
    /// The model changed, and every model keeps a cache of its own.
    ModelSwitch,
    /// The conversation was compacted, which rewrites the prompt the cache was built on.
    Compaction,
    /// More time passed than the cache lives: five minutes, or an hour when it was written for one.
    IdleGap,
    /// Cortex gave the agent text between the two responses, and nothing above explains the break.
    HookText,
}

/// One response that lost the cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheBreak {
    /// The session whose conversation it is: the receipt's own, or one of its subagents'.
    pub session_id: Identifier,
    /// The response's number in that session's transcript.
    pub turn: u32,
    /// When it was written, in UTC milliseconds.
    pub at_ms: i64,
    /// What most likely caused it, or `None` when nothing known explains it.
    pub cause: Option<BreakCause>,
}

/// What happened to the conversations between their responses, as the store recorded it.
#[derive(Debug, Clone, Copy)]
pub struct BreakContext<'a> {
    /// When each session compacted its conversation, in UTC milliseconds, by session; a session
    /// left out never did.
    pub compactions_at_ms: &'a BTreeMap<Identifier, Vec<i64>>,
    /// When Cortex gave text that reached the agent, in UTC milliseconds, in any order.
    pub texts_given_at_ms: &'a [i64],
}

/// The responses of `calls` that lost the cache, earliest first, each conversation's responses
/// taken in turn order and apart from the others'. The first response of a conversation never
/// breaks, having none before it to lose a cache from.
#[must_use]
pub fn cache_breaks(calls: &[UsageRow], context: &BreakContext<'_>) -> Vec<CacheBreak> {
    let mut conversations: BTreeMap<&Identifier, Vec<&UsageRow>> = BTreeMap::new();
    for row in calls {
        conversations.entry(&row.session_id).or_default().push(row);
    }
    let mut breaks: Vec<CacheBreak> = Vec::new();
    for (session_id, mut rows) in conversations {
        rows.sort_by_key(|row| (row.turn, row.at_ms));
        let compactions = context
            .compactions_at_ms
            .get(session_id)
            .map_or(&[][..], Vec::as_slice);
        let mut between = Between {
            compactions_at_ms: compactions,
            texts_given_at_ms: context.texts_given_at_ms,
            lifetime: Lifetime::default(),
        };
        for pair in rows.windows(2) {
            let [previous, current] = pair else {
                continue;
            };
            between.lifetime.note(previous);
            if is_break(previous, current) {
                breaks.push(CacheBreak {
                    session_id: current.session_id.clone(),
                    turn: current.turn,
                    at_ms: current.at_ms,
                    cause: between.cause_of(previous, current),
                });
            }
        }
    }
    breaks.sort_by(|left, right| {
        (left.at_ms, &left.session_id, left.turn).cmp(&(right.at_ms, &right.session_id, right.turn))
    });
    breaks
}

/// Whether `current` lost the cache `previous` left: it wrote more than half its cached input
/// and read less than half of what `previous` read. A `previous` that read nothing left no cache
/// to lose.
fn is_break(previous: &UsageRow, current: &UsageRow) -> bool {
    let written = u128::from(current.cache_write);
    let read = u128::from(current.cache_read);
    let is_rebuilt = written * PER_MILLE > (written + read) * WRITE_SHARE_BREAK_PER_MILLE;
    let is_lost = read * PER_MILLE < u128::from(previous.cache_read) * READ_KEPT_BREAK_PER_MILLE;
    is_rebuilt && is_lost
}

/// What happened to one conversation that may explain its breaks.
struct Between<'a> {
    /// When it was compacted, in UTC milliseconds.
    compactions_at_ms: &'a [i64],
    /// When Cortex gave text that reached the agent, in UTC milliseconds.
    texts_given_at_ms: &'a [i64],
    /// How long its cache lives, from the responses so far.
    lifetime: Lifetime,
}

/// How long a conversation's cache lives, from the cache writes of its responses so far.
#[derive(Debug, Default)]
struct Lifetime {
    /// Whether a response has written to the cache.
    has_written: bool,
    /// Whether a response wrote to the cache for five minutes, or did not say for how long.
    has_short_write: bool,
}

impl Lifetime {
    /// Takes in the cache writes of `row`, the next response.
    fn note(&mut self, row: &UsageRow) {
        if row.cache_write > 0 {
            self.has_written = true;
            self.has_short_write |= row.cache_write_1h != Some(row.cache_write);
        }
    }

    /// The lifetime in milliseconds: an hour when every write was for an hour, else five minutes.
    fn ms(&self) -> i64 {
        if self.has_written && !self.has_short_write {
            ONE_HOUR_LIFETIME_MS
        } else {
            FIVE_MINUTE_LIFETIME_MS
        }
    }
}

impl Between<'_> {
    /// The most likely cause of the break at `current`, the first that holds of a model switch, a
    /// compaction, an idle gap and text Cortex gave: they run from the one that always loses the
    /// cache to the one that may only have.
    fn cause_of(&self, previous: &UsageRow, current: &UsageRow) -> Option<BreakCause> {
        // A time on either edge of the span counts: the hook's clock and the transcript's do not
        // agree to the millisecond.
        let is_between = |times: &[i64]| {
            times
                .iter()
                .any(|at_ms| (previous.at_ms..=current.at_ms).contains(at_ms))
        };
        if is_model_switch(previous, current) {
            return Some(BreakCause::ModelSwitch);
        }
        if is_between(self.compactions_at_ms) {
            return Some(BreakCause::Compaction);
        }
        if current.at_ms.saturating_sub(previous.at_ms) > self.lifetime.ms() {
            return Some(BreakCause::IdleGap);
        }
        is_between(self.texts_given_at_ms).then_some(BreakCause::HookText)
    }
}

/// Whether the two responses name different models; one that names none switched nothing known.
fn is_model_switch(previous: &UsageRow, current: &UsageRow) -> bool {
    matches!(
        (&previous.model, &current.model),
        (Some(before), Some(after)) if before != after
    )
}

#[cfg(test)]
#[path = "cache_breaks_tests.rs"]
mod tests;
