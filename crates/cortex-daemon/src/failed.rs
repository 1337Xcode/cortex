//! The latest failed check of each recent session, as its failure map grounded it: the nodes its
//! stack ran through in the repository and the runners whose output was read. Kept in memory only,
//! never stored, for the session's next prompt to seed from; that prompt takes it, a later failure
//! of the session replaces it, and a run of one of its runners that passed drops it, since the
//! failure is gone.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use cortex_core::Identifier;
use cortex_store::NodeId;

/// How many sessions' failures are kept, the oldest dropped first: a few sessions work at once, and
/// a failure older than a few dozen others is no longer what a prompt is about.
const MAX_SESSIONS: usize = 64;

/// How long a failure stays a seed, in milliseconds: a prompt half an hour after the run is about
/// something else, or the run has been repeated since.
pub(crate) const FAILURE_SEED_MS: i64 = 30 * 60 * 1_000;

/// One session's latest failure.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Latest {
    /// The session.
    session: Identifier,
    /// When the failed run's result came, in UTC milliseconds.
    at_ms: i64,
    /// The runners the failed line ran, whose passing run drops it.
    runners: Vec<&'static str>,
    /// The grounded frames' nodes, each failure's innermost first.
    nodes: Vec<NodeId>,
}

/// The latest failure of each recent session.
#[derive(Debug, Default)]
pub(crate) struct Failed {
    /// Each session's latest, the one kept longest ago first.
    latest: Mutex<VecDeque<Latest>>,
}

impl Failed {
    /// Keeps `nodes`, the grounded frames of a run of `runners` that failed at `at_ms`, as
    /// `session`'s latest failure, in place of any before it.
    pub(crate) fn keep(
        &self,
        session: &Identifier,
        at_ms: i64,
        (runners, nodes): (Vec<&'static str>, Vec<NodeId>),
    ) {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        latest.retain(|kept| kept.session != *session);
        if latest.len() == MAX_SESSIONS {
            latest.pop_front();
        }
        latest.push_back(Latest {
            session: session.clone(),
            at_ms,
            runners,
            nodes,
        });
    }

    /// Takes `session`'s latest failure for its prompt at `now_ms`: its nodes, or none when it has
    /// none or the failure is older than [`FAILURE_SEED_MS`]. Either way it is no longer kept.
    pub(crate) fn take(&self, session: &Identifier, now_ms: i64) -> Vec<NodeId> {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(at) = latest.iter().position(|kept| kept.session == *session) else {
            return Vec::new();
        };
        let Some(kept) = latest.remove(at) else {
            return Vec::new();
        };
        if now_ms.saturating_sub(kept.at_ms) > FAILURE_SEED_MS {
            return Vec::new();
        }
        kept.nodes
    }

    /// Drops `session`'s latest failure when `passed`, the runners of a line that exited 0, holds
    /// one of its runners: that run passed, and the failure is gone.
    pub(crate) fn passed(&self, session: &Identifier, passed: &[&str]) {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        latest.retain(|kept| {
            kept.session != *session || !kept.runners.iter().any(|runner| passed.contains(runner))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// Five nodes of a store built for the test.
    fn five_nodes() -> TestResult<(tempfile::TempDir, Vec<NodeId>)> {
        let root = tempfile::tempdir()?;
        let mut store = writer(root.path())?;
        let mut nodes = Vec::new();
        for name in ["a.py", "b.py", "c.py", "d.py", "e.py"] {
            nodes.push(add_file(&mut store, name, 1)?);
        }
        drop(store);
        Ok((root, nodes))
    }

    /// A session's failure is taken once, by its next prompt; a later failure takes the place of
    /// an earlier one; one older than half an hour seeds nothing.
    #[test]
    fn a_failure_seeds_the_next_prompt_once() -> TestResult {
        let (_root, nodes) = five_nodes()?;
        let pick = |at: &[usize]| -> Vec<NodeId> {
            at.iter().filter_map(|at| nodes.get(*at).copied()).collect()
        };
        let failed = Failed::default();
        let (one, two) = (Identifier::new("s1")?, Identifier::new("s2")?);
        failed.keep(&one, 1_000, (vec!["pytest"], pick(&[0, 1])));
        failed.keep(&one, 2_000, (vec!["pytest"], pick(&[2])));
        failed.keep(&two, 2_000, (vec!["cargo"], pick(&[3])));
        assert_eq!(failed.take(&one, 3_000), pick(&[2]));
        assert_eq!(failed.take(&one, 3_000), pick(&[]));
        assert_eq!(failed.take(&two, 2_000 + FAILURE_SEED_MS + 1), pick(&[]));
        Ok(())
    }

    /// A passing run of one of the failure's runners drops it; another runner's, or another
    /// session's, does not.
    #[test]
    fn a_passing_run_drops_the_failure() -> TestResult {
        let (_root, nodes) = five_nodes()?;
        let pick = |at: &[usize]| -> Vec<NodeId> {
            at.iter().filter_map(|at| nodes.get(*at).copied()).collect()
        };
        let failed = Failed::default();
        let (one, two) = (Identifier::new("s1")?, Identifier::new("s2")?);
        failed.keep(&one, 1_000, (vec!["pytest"], pick(&[0])));
        failed.passed(&one, &["cargo"]);
        failed.passed(&two, &["pytest"]);
        failed.keep(&two, 1_000, (vec!["go"], pick(&[4])));
        assert_eq!(failed.take(&one, 1_500), pick(&[0]));
        failed.keep(&one, 1_000, (vec!["pytest"], pick(&[0])));
        failed.passed(&one, &["pytest"]);
        assert_eq!(failed.take(&one, 1_500), pick(&[]));
        assert_eq!(failed.take(&two, 1_500), pick(&[4]));
        Ok(())
    }
}
