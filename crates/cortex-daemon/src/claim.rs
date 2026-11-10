//! What an answer claimed in the daemon's memory while it was worked out, so that one text is not
//! given twice: the pairs a task was warned of, a compaction recapped, a task given strict mode's
//! reason to go on, the coordination notices a session was given, a handoff given to a first
//! prompt, a file a task's read named as the one earlier tasks read next. An answer whose text
//! never reaches its hook (worked out too late, written after the hook stopped reading, or not
//! written at all) gives its claim back, so that a later edit, start, read or stop is told what
//! this one was not.

use std::sync::Arc;

use cortex_core::{Identifier, NoticeSubject, Ulid};
use cortex_store::{HandoffId, NodeId, NoticeId};

use crate::noticed::Noticed;
use crate::once::ClaimedOnce;
use crate::recapped::Recapped;
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
    /// Session `.0`, given the waiting notices `.1` and told lines about `.2`.
    Noticed(Identifier, Vec<NoticeId>, Vec<NoticeSubject>),
    /// Handoff `.0`, given to a first prompt.
    Handed(HandoffId),
    /// The file `.1`, named to task `.0` as the one earlier tasks read next.
    ReadNext(Ulid, NodeId),
}

/// Where the daemon keeps what answers claimed.
#[derive(Debug, Default, Clone)]
pub(crate) struct Claims {
    /// The pairs each recent task was warned of.
    pub(crate) told: Arc<Told>,
    /// The compactions each recent session's agent reported, and those recapped.
    pub(crate) recapped: Arc<Recapped>,
    /// The tasks strict mode gave a reason to go on.
    pub(crate) stopped: Arc<ClaimedOnce<Ulid>>,
    /// The coordination notices each recent session was given.
    pub(crate) noticed: Arc<Noticed>,
    /// The handoffs given to first prompts.
    pub(crate) handed: Arc<ClaimedOnce<HandoffId>>,
    /// The files named to tasks as the ones earlier tasks read next, with their tasks: two reads
    /// answered at once name each file once, before either's record is stored.
    pub(crate) read_next: Arc<ClaimedOnce<(Ulid, NodeId)>>,
}

impl Claims {
    /// Gives `claim` back: its pairs untold, its compaction not recapped, its task not given its
    /// reason, its notices waiting again and its lines untold, its handoff open to give again, its
    /// file open to name again.
    pub(crate) fn release(&self, claim: &Claim) {
        match claim {
            Claim::Warned(task_id, pairs) => self.told.forget(*task_id, pairs),
            Claim::Recapped(session, start) => self.recapped.release(session, *start),
            Claim::Stopped(task_id) => self.stopped.release(task_id),
            Claim::Handed(handoff) => self.handed.release(handoff),
            Claim::ReadNext(task_id, node) => self.read_next.release(&(*task_id, *node)),
            Claim::Noticed(session, delivered, told) => {
                self.noticed.release(session, (delivered, told));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use cortex_core::WarningKind;
    use cortex_retrieve::{Warned, Warning};
    use cortex_store::{HandoffEnd, HandoffRecord};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// Each claim given back can be made again: a pair told and released is told again, a
    /// compaction recapped and released is recapped again, a task stopped and released is given its
    /// reason again, a handoff given and released is given again, a file named and released is
    /// named again; a claim of another task, session, start or file gives nothing back.
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

        let mut transaction = store.transaction()?;
        let handoff = transaction.leave_handoff(&HandoffRecord {
            from_session: session,
            task_id: task,
            end: HandoffEnd::Left,
            edited: vec![source],
            read: Vec::new(),
            open_warnings: Vec::new(),
            last_failure: None,
            created_at_ms: 1_000,
        })?;
        transaction.commit()?;
        assert!(claims.handed.claim(handoff));
        assert!(!claims.handed.claim(handoff));
        claims.release(&Claim::Handed(handoff));
        assert!(claims.handed.claim(handoff));

        assert!(claims.read_next.claim((task, target)));
        assert!(!claims.read_next.claim((task, target)));
        claims.release(&Claim::ReadNext(task, source));
        assert!(!claims.read_next.claim((task, target)));
        claims.release(&Claim::ReadNext(task, target));
        assert!(claims.read_next.claim((task, target)));
        Ok(())
    }
}
