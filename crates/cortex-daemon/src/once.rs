//! Things an answer may give once only, claimed under one lock so that two answers worked out at
//! once give each thing once: the tasks strict mode gave a reason to go on when their agent
//! stopped, and the handoffs given to a session's first prompt. An answer whose text never reaches
//! its hook gives its claim back, so a later answer can give the thing again.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

/// How many things are kept, the one claimed longest ago dropped first: the things claimed are a
/// few per session working at once, so a few dozen cover them all. A thing dropped could be given a
/// second time, which a claim that old has earned.
const MAX_CLAIMED: usize = 64;

/// The things of one kind that were given.
#[derive(Debug)]
pub(crate) struct ClaimedOnce<K> {
    /// Each thing, the one claimed longest ago first.
    claimed: Mutex<VecDeque<K>>,
}

impl<K> Default for ClaimedOnce<K> {
    fn default() -> Self {
        Self {
            claimed: Mutex::new(VecDeque::new()),
        }
    }
}

impl<K: PartialEq> ClaimedOnce<K> {
    /// Whether `thing` is to be given: true the first time, which is kept in the same step.
    pub(crate) fn claim(&self, thing: K) -> bool {
        let mut claimed = self.claimed.lock().unwrap_or_else(PoisonError::into_inner);
        if claimed.contains(&thing) {
            return false;
        }
        if claimed.len() == MAX_CLAIMED {
            claimed.pop_front();
        }
        claimed.push_back(thing);
        true
    }

    /// Gives back `thing`'s claim: it never reached the agent.
    pub(crate) fn release(&self, thing: &K) {
        let mut claimed = self.claimed.lock().unwrap_or_else(PoisonError::into_inner);
        claimed.retain(|kept| kept != thing);
    }
}

#[cfg(test)]
mod tests {
    use super::{ClaimedOnce, MAX_CLAIMED};

    /// A thing is claimed once, another apart; given back, it can be claimed again; once
    /// [`MAX_CLAIMED`] more were claimed, the first is forgotten.
    #[test]
    fn a_thing_is_claimed_once() {
        let once = ClaimedOnce::default();
        assert!(once.claim(1_u64));
        assert!(!once.claim(1));
        assert!(once.claim(2));
        once.release(&2);
        assert!(once.claim(2));
        for thing in (100_u64..).take(MAX_CLAIMED) {
            assert!(once.claim(thing));
        }
        assert!(once.claim(1));
    }
}
