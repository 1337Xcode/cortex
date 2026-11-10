//! The coordination notices each recent session was given, kept from the moment they are decided
//! until the writer has recorded them. The store learns of a notice given only once the writer
//! records it, after the answer is written: two hooks of one session answered close together (an
//! agent's tools often run in parallel) would otherwise both find a waiting notice and both give
//! it, or both find a line untold and both tell it. Deciding under this lock and keeping what was
//! decided in the same step gives each notice once, whatever the writer is doing. A line is told
//! by what it is about, not by its words: one happening told from the other side, or in other
//! words, is told once.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use cortex_coord::Notice;
use cortex_core::{Identifier, NoticeSubject};
use cortex_store::{NoticeId, NoticeRow};

/// How many sessions' notices are kept, the one decided for longest ago dropped first; by then
/// the writer has long recorded them.
const MAX_SESSIONS: usize = 64;

/// How many of one session's notices are kept, the oldest dropped first.
const MAX_PER_SESSION: usize = 256;

/// What one session was given lately that the store may not hold yet.
#[derive(Debug, Default)]
struct Given {
    /// The waiting notices given to it.
    delivered: VecDeque<NoticeId>,
    /// What the lines it was told at once were about, each with when.
    told: VecDeque<(NoticeSubject, i64)>,
}

/// What a decision gives one session: the waiting notices to deliver and the lines to tell.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Giving {
    /// The waiting notices it gets, oldest first.
    pub(crate) delivered: Vec<NoticeRow>,
    /// The lines it is told.
    pub(crate) told: Vec<Notice>,
}

/// The coordination notices each recent session was given.
#[derive(Debug, Default)]
pub(crate) struct Noticed {
    /// Each session with what it was given, the one decided for longest ago first.
    sessions: Mutex<VecDeque<(Identifier, Given)>>,
}

impl Noticed {
    /// Decides what `session` is given at `now_ms` of `waiting`, the notices the store says wait
    /// for it, and of `lines`, those it could be told, by `pick`, which sees only what was not
    /// given yet: a waiting notice given since the store was read is left out first, and so is a
    /// line that says no more than one already sent: one whose subject is covered by one in `sent`
    /// (what the notices the store says the session was sent or given at `since_ms` or later are
    /// about, those still waiting among them) or by that of a line told here since then. What
    /// `pick` gives is kept as given, all while no other decision runs.
    pub(crate) fn deciding(
        &self,
        session: &Identifier,
        (waiting, lines, sent): (Vec<NoticeRow>, Vec<Notice>, &[NoticeSubject]),
        (since_ms, now_ms): (i64, i64),
        pick: impl FnOnce(Giving) -> Giving,
    ) -> Giving {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        // The session decided for now goes to the back, so that the front is the one decided for
        // longest ago.
        let kept = sessions
            .iter()
            .position(|(kept, _)| kept == session)
            .and_then(|at| sessions.remove(at));
        let entry = kept.unwrap_or_else(|| (session.clone(), Given::default()));
        if sessions.len() == MAX_SESSIONS {
            sessions.pop_front();
        }
        sessions.push_back(entry);
        let Some((_, given)) = sessions.back_mut() else {
            return Giving::default();
        };
        let told: Vec<Notice> = lines
            .into_iter()
            .filter(|line| {
                !sent.iter().any(|sent| sent.covers(&line.subject))
                    && !given
                        .told
                        .iter()
                        .any(|(told, at_ms)| *at_ms >= since_ms && told.covers(&line.subject))
            })
            .collect();
        let open = Giving {
            delivered: waiting
                .into_iter()
                .filter(|notice| !given.delivered.contains(&notice.id))
                .collect(),
            told,
        };
        let giving = pick(open);
        for notice in &giving.delivered {
            keep(&mut given.delivered, notice.id);
        }
        for line in &giving.told {
            keep(&mut given.told, (line.subject.clone(), now_ms));
        }
        giving
    }

    /// The waiting notices `session` was given that the store may not know of yet.
    pub(crate) fn given_to(&self, session: &Identifier) -> Vec<NoticeId> {
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        sessions
            .iter()
            .find(|(kept, _)| kept == session)
            .map(|(_, given)| given.delivered.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Forgets that `session` was given `delivered` and told lines about `told`: they never reached
    /// its agent, so the notices wait again and the lines can be told again.
    pub(crate) fn release(
        &self,
        session: &Identifier,
        (delivered, told): (&[NoticeId], &[NoticeSubject]),
    ) {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, given)) = sessions.iter_mut().find(|(kept, _)| kept == session) {
            given.delivered.retain(|id| !delivered.contains(id));
            given.told.retain(|(line, _)| !told.contains(line));
        }
    }
}

/// Keeps `value` at the back of `kept`, dropping the oldest past [`MAX_PER_SESSION`].
fn keep<T>(kept: &mut VecDeque<T>, value: T) {
    if kept.len() == MAX_PER_SESSION {
        kept.pop_front();
    }
    kept.push_back(value);
}

#[cfg(test)]
mod tests {
    use cortex_store::{NoticeRecord, Reader};

    use super::*;
    use crate::test_store::{TestResult, writer};

    /// Two notices waiting for `s1`, as the store gives them back; the folder holding the store
    /// goes with them.
    fn two_waiting() -> TestResult<(tempfile::TempDir, Vec<NoticeRow>)> {
        let root = tempfile::tempdir()?;
        let mut store = writer(root.path())?;
        let session = Identifier::new("s1")?;
        let mut transaction = store.transaction()?;
        for (text, at_ms) in [("first", 1), ("second", 2)] {
            let notice = NoticeRecord {
                to_session: session.clone(),
                about: subject(&format!("about {text}")),
                text: text.to_owned(),
                at_ms,
            };
            transaction.queue_notice(&notice, 0)?;
        }
        transaction.commit()?;
        let reader = Reader::open(&root.path().join(".cortex"), 0)?;
        let rows = reader.queries().notices_waiting(&session, 0)?;
        Ok((root, rows))
    }

    /// A subject the store could have kept, `text` whole.
    fn subject(text: &str) -> NoticeSubject {
        NoticeSubject::from_stored(text.to_owned())
    }

    /// A line about `about` saying `text`.
    fn said(about: &str, text: &str) -> Vec<Notice> {
        vec![Notice {
            subject: subject(about),
            text: text.to_owned(),
        }]
    }

    /// Two hooks of one session decided one after the other, both from the same read of the
    /// store: the first is given both waiting notices and its line, the second neither, nor the
    /// same line in other words. A line told at 10 is not told again with the window opening at 5,
    /// and is with it opening at 11; nor is one about what the store says was sent, in whatever
    /// words it was sent. Another session is given its own. Given back, they are given again.
    #[test]
    fn each_notice_is_given_once_until_given_back() -> TestResult {
        let (_store, waiting) = two_waiting()?;
        let noticed = Noticed::default();
        let (s1, s2) = (Identifier::new("s1")?, Identifier::new("s2")?);
        let line = || said("about the line", "a line");
        let reworded = || said("about the line", "the line in other words");
        let about = || vec![subject("about the line")];
        let all = |open: Giving| open;
        let first = noticed.deciding(&s1, (waiting.clone(), line(), &[]), (5, 10), all);
        assert_eq!(
            first,
            Giving {
                delivered: waiting.clone(),
                told: line(),
            }
        );
        let again = noticed.deciding(&s1, (waiting.clone(), reworded(), &[]), (5, 12), all);
        assert_eq!(again, Giving::default());
        let later = noticed.deciding(&s1, (Vec::new(), line(), &[]), (11, 30), all);
        assert_eq!(later.told, line());
        // A line about what the store says was sent since the window opened is not told, though
        // this daemon never told it (it started since).
        let sent_before = noticed.deciding(&s2, (Vec::new(), reworded(), &about()), (0, 10), all);
        assert_eq!(sent_before.told, Vec::<Notice>::new());
        let other = noticed.deciding(&s2, (waiting.clone(), line(), &[]), (5, 10), all);
        assert_eq!(other.delivered, waiting);

        let ids: Vec<NoticeId> = waiting.iter().map(|notice| notice.id).collect();
        noticed.release(&s1, (&ids, &about()));
        let given_back = noticed.deciding(&s1, (waiting.clone(), line(), &[]), (5, 40), all);
        assert_eq!(
            given_back,
            Giving {
                delivered: waiting,
                told: line(),
            }
        );
        Ok(())
    }

    /// What `pick` leaves out is not kept as given: a notice it did not give is given later.
    #[test]
    fn what_was_not_picked_waits() -> TestResult {
        let (_store, waiting) = two_waiting()?;
        let noticed = Noticed::default();
        let s1 = Identifier::new("s1")?;
        let first_only = |open: Giving| Giving {
            delivered: open.delivered.into_iter().take(1).collect(),
            told: Vec::new(),
        };
        let first = noticed.deciding(&s1, (waiting.clone(), Vec::new(), &[]), (0, 1), first_only);
        assert_eq!(first.delivered, waiting.get(..1).unwrap_or_default());
        let rest = noticed.deciding(&s1, (waiting.clone(), Vec::new(), &[]), (0, 2), |open| open);
        assert_eq!(rest.delivered, waiting.get(1..).unwrap_or_default());
        Ok(())
    }
}
