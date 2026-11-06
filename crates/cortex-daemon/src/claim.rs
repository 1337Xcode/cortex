//! What an answer claimed in the daemon's memory while it was worked out, so that one text is not
//! given twice: the pairs a task was warned of, a compaction recapped, a task given strict mode's
//! reason to go on. An answer whose text never reaches its hook (worked out too late, written after
//! the hook stopped reading, or not written at all) gives its claim back, so that a later edit,
//! start or stop is told what this one was not.

use std::sync::Arc;

use cortex_core::{Identifier, Ulid};

use crate::recapped::Recapped;
use crate::stopped::Stopped;
use crate::told::{Pair, Told};

/// What one answer claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Claim {
    /// The pairs task `.0` was warned of.
    Warned(Ulid, Vec<Pair>),
    /// The compaction of session `.0` recapped at its start `.1`.
    Recapped(Identifier, Ulid),
    /// Task `.0`, given its reason to go on.
    Stopped(Ulid),
}

/// Where the daemon keeps what answers claimed.
#[derive(Debug, Default, Clone)]
pub(crate) struct Claims {
    /// The pairs each recent task was warned of.
    pub(crate) told: Arc<Told>,
    /// The compactions each recent session's agent reported, and those recapped.
    pub(crate) recapped: Arc<Recapped>,
    /// The tasks strict mode gave a reason to go on.
    pub(crate) stopped: Arc<Stopped>,
}

impl Claims {
    /// Gives `claim` back: its pairs untold, its compaction not recapped, its task not given its
    /// reason.
    pub(crate) fn release(&self, claim: &Claim) {
        match claim {
            Claim::Warned(task_id, pairs) => self.told.forget(*task_id, pairs),
            Claim::Recapped(session, start) => self.recapped.release(session, *start),
            Claim::Stopped(task_id) => self.stopped.release(*task_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::WarningKind;
    use cortex_retrieve::{Warned, Warning};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// Each claim given back can be made again: a pair told and released is told again, a
    /// compaction recapped and released is recapped again, a task stopped and released is given its
    /// reason again; a claim of another task, session or start gives nothing back.
    #[test]
    fn a_claim_given_back_can_be_made_again() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut store = writer(root.path())?;
        let source = add_file(&mut store, "calc/ops.py", 1)?;
        let target = add_file(&mut store, "main.py", 1)?;
        let claims = Claims::default();
        let task = Ulid::from_seed(1, b"task");
        let pair = (WarningKind::CallSite, source, target);
        let warn = || Warned {
            given: vec![Warning {
                kind: pair.0,
                source: pair.1,
                targets: vec![pair.2],
                strength: 1.0,
                line: "a warning".to_owned(),
            }],
            text: Some("[cortex] a warning\n".to_owned()),
        };
        let told_now = |claims: &Claims| {
            let mut seen = false;
            claims.told.deciding(task, |told| {
                seen = told.contains(&pair);
                Warned::default()
            });
            seen
        };
        claims.told.deciding(task, |_| warn());
        assert!(told_now(&claims));
        claims.release(&Claim::Warned(Ulid::from_seed(2, b"task"), vec![pair]));
        assert!(told_now(&claims));
        claims.release(&Claim::Warned(task, vec![pair]));
        assert!(!told_now(&claims));

        let session = Identifier::new("s1")?;
        let start = Ulid::from_seed(3, b"start");
        claims.recapped.compacted(&session);
        assert!(claims.recapped.claim(&session, start));
        assert!(!claims.recapped.claim(&session, start));
        claims.release(&Claim::Recapped(Identifier::new("s2")?, start));
        assert!(!claims.recapped.claim(&session, start));
        claims.release(&Claim::Recapped(session.clone(), start));
        assert!(claims.recapped.claim(&session, start));

        assert!(claims.stopped.claim(task));
        claims.release(&Claim::Stopped(Ulid::from_seed(2, b"task")));
        assert!(!claims.stopped.claim(task));
        claims.release(&Claim::Stopped(task));
        assert!(claims.stopped.claim(task));
        Ok(())
    }
}
