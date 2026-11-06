//! The compactions each recent session's agent reported, and the one it was last given its recap
//! for. A compaction is noted as the agent reports it (Claude Code's and Codex's `PreCompact`),
//! before the writer records it, so the start that follows it finds it whatever the writer is
//! doing; and the recap is claimed under the same lock it is checked under, so two starts of one
//! compaction answered at once give one recap.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use cortex_core::{Identifier, Ulid};

/// How many sessions are kept, the one heard of longest ago dropped first: a few dozen cover every
/// session working at once. A session dropped, or one from before the daemon started, has heard of
/// no compaction, so each of its starts after one counts as a compaction of its own.
const MAX_SESSIONS: usize = 64;

/// One compaction of a session's context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compaction {
    /// The session's compaction the daemon heard of, by its number among them, from 1.
    Heard(u64),
    /// A start after a compaction the daemon heard nothing of, by the start's event: nothing tells
    /// two such starts apart.
    Started(Ulid),
}

/// What is kept of one session.
#[derive(Debug)]
struct Session {
    /// The session.
    id: Identifier,
    /// How many compactions the daemon heard of.
    heard: u64,
    /// The compaction it was last given its recap for.
    recapped: Option<Compaction>,
}

/// The compactions of each recent session, and those recapped.
#[derive(Debug, Default)]
pub(crate) struct Recapped {
    /// Each session, the one heard of longest ago first.
    sessions: Mutex<VecDeque<Session>>,
}

impl Recapped {
    /// Notes that `session`'s agent is compacting its context.
    pub(crate) fn compacted(&self, session: &Identifier) {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = session_in(&mut sessions, session);
        kept.heard = kept.heard.saturating_add(1);
    }

    /// Whether the start `start` of `session`, after a compaction, is to be given its recap: true
    /// the first time a start of the session's latest compaction asks, which is kept as recapped
    /// in the same step. With no compaction heard of, the start is a compaction of its own.
    pub(crate) fn claim(&self, session: &Identifier, start: Ulid) -> bool {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = session_in(&mut sessions, session);
        let compaction = match kept.heard {
            0 => Compaction::Started(start),
            heard => Compaction::Heard(heard),
        };
        if kept.recapped == Some(compaction) {
            return false;
        }
        kept.recapped = Some(compaction);
        true
    }
}

/// The entry of `session` among `sessions`, made when there is none, the oldest dropped to keep
/// [`MAX_SESSIONS`].
fn session_in<'a>(sessions: &'a mut VecDeque<Session>, session: &Identifier) -> &'a mut Session {
    let at = match sessions.iter().position(|kept| kept.id == *session) {
        Some(at) => at,
        None => {
            if sessions.len() == MAX_SESSIONS {
                sessions.pop_front();
            }
            sessions.push_back(Session {
                id: session.clone(),
                heard: 0,
                recapped: None,
            });
            sessions.len() - 1
        }
    };
    #[allow(
        clippy::indexing_slicing,
        reason = "`at` is the position of an entry found or pushed under the same lock"
    )]
    &mut sessions[at]
}

#[cfg(test)]
mod tests {
    use super::{MAX_SESSIONS, Recapped};
    use cortex_core::{Identifier, Ulid};

    /// A compaction heard of is recapped once, however many starts follow it; the next one heard
    /// of again once. A start with no compaction heard of is one of its own: a second start with
    /// the same event is not recapped again, another is. Sessions are apart. Once
    /// [`MAX_SESSIONS`] more sessions were heard of, the first is forgotten.
    #[test]
    fn each_compaction_is_recapped_once() -> Result<(), Box<dyn std::error::Error>> {
        let recapped = Recapped::default();
        let first = Identifier::new("s1")?;
        let (start, again) = (Ulid::from_seed(1, b"start"), Ulid::from_seed(2, b"start"));
        assert!(recapped.claim(&first, start));
        assert!(!recapped.claim(&first, start));
        assert!(recapped.claim(&first, again));
        recapped.compacted(&first);
        assert!(recapped.claim(&first, start));
        assert!(!recapped.claim(&first, again));
        recapped.compacted(&first);
        assert!(recapped.claim(&first, start));
        assert!(!recapped.claim(&first, start));
        let second = Identifier::new("s2")?;
        recapped.compacted(&second);
        assert!(recapped.claim(&second, start));
        for session in 0..MAX_SESSIONS {
            recapped.compacted(&Identifier::new(&format!("x{session}"))?);
        }
        assert!(recapped.claim(&first, start));
        assert!(!recapped.claim(&first, start));
        Ok(())
    }
}
