//! What a person or an agent asks the writer for, as one kind of job: a note, the user's word on
//! the facts, a claim made or released. Each job is carried out in a transaction of its own and
//! answered only once that transaction is committed, so what the asker is told was done is
//! written; a job the writer did
//! not start in time is dropped unwritten, so nothing the asker was told failed is done after all.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use cortex_coord::Timing;
use cortex_core::Identifier;
use cortex_core::ipc::{FactsAsk, Note, ResponseBody};
use cortex_store::{StoreError, WriteTx, Writer};

use crate::{claims, facts};

/// How long a request may wait for the writer before it is dropped unwritten: long enough for a
/// batch of the index to end, short enough that the person who typed it is told.
pub(crate) const ASK_START_WITHIN: Duration = Duration::from_secs(10);

/// How long the writer has to answer a request it took: one transaction of a few rows.
pub(crate) const ASK_WRITE_WITHIN: Duration = Duration::from_secs(5);

/// How long before an agent's note or a claim the session that made it was last seen at work, at
/// most, in milliseconds: neither an MCP call nor a command carries the session, so it is taken
/// to be the session whose latest event came last within these five minutes, longer than an
/// agent's turn spends between two tool calls.
pub(crate) const AT_WORK_WITHIN_MS: i64 = 5 * 60 * 1_000;

/// The sessions at work when a request is carried out, a request naming none: the one with the
/// latest event, which holds a claim; and that one again only when no other session was at work,
/// the one whose open task an agent's note is taken to be (of two at work, a note names neither,
/// and is filed in no task).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AtWork {
    /// The session with the latest event.
    pub(crate) latest: Option<Identifier>,
    /// The only session at work, when there is one.
    pub(crate) sole: Option<Identifier>,
}

/// A request for the writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ask {
    /// A fact noted, by the user or an agent.
    Note(Note),
    /// The user's word on the facts.
    Facts(FactsAsk),
    /// Files, by repository path, claimed for the session at work.
    Claim(Vec<String>),
    /// The claims on files, by repository path, released; every claim when there is none.
    Release(Vec<String>),
}

/// A request for the writer, with where to answer it and until when to start on it.
#[derive(Debug)]
pub(crate) struct AskJob {
    /// What is asked.
    request: Ask,
    /// When the asker stops waiting for the writer to start: a job taken later is dropped
    /// unwritten.
    start_by: Instant,
    /// Where the writer answers: the response, or why there is none.
    answered: SyncSender<Answered>,
}

/// The writer's answer to a request it took: the response, or why there is none.
pub(crate) type Answered = Result<ResponseBody, NotDone>;

/// Why a request the writer took was not carried out, as the asker is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotDone {
    /// It was refused: it named something wrongly, in words the asker reads.
    Refused(String),
    /// The store failed it and wrote nothing. The error goes to the writer, which logs it and
    /// keeps it among the daemon's errors; it is not told to the asker, as its words can name the
    /// database's path.
    Failed,
}

/// Why a request was refused.
#[derive(Debug)]
pub(crate) enum Refused {
    /// The request named something wrongly, in words the asker reads.
    Named(String),
    /// The store failed.
    Store(StoreError),
}

impl From<StoreError> for Refused {
    /// The store's failure, as it is.
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl AskJob {
    /// A job for `request`, to be started within [`ASK_START_WITHIN`], and where its answer
    /// arrives.
    pub(crate) fn new(request: Ask) -> (Self, Receiver<Answered>) {
        let (answered, answer) = mpsc::sync_channel(1);
        let job = Self {
            request,
            start_by: Instant::now() + ASK_START_WITHIN,
            answered,
        };
        (job, answer)
    }

    /// Answers the job as one the store failed, writing nothing, for the tests of what an asker
    /// does with that answer.
    #[cfg(test)]
    pub(crate) fn fail_for_tests(self) {
        drop(self.answered.send(Err(NotDone::Failed)));
    }

    /// Carries out the job at `now_ms` with `writer`, in a transaction of its own, and answers it
    /// only once that transaction is committed. A refusal rolls the transaction back and is the
    /// answer; a job that waited too long is dropped unanswered and nothing is written.
    ///
    /// `at_work` holds the sessions at work: the latest holds a claim, and an agent's note is taken
    /// to be in the open task of the only one; positions hold as `timing` says.
    ///
    /// # Errors
    ///
    /// The store's errors, the commit's among them, which are also the job's answer.
    pub(crate) fn carry_out(
        self,
        writer: &mut Writer,
        (now_ms, at_work, timing): (i64, &AtWork, &Timing),
    ) -> Result<(), StoreError> {
        if Instant::now() > self.start_by {
            return Ok(());
        }
        let done = writer
            .transaction()
            .map_err(Refused::from)
            .and_then(|mut transaction| {
                let answer = answer(&mut transaction, &self.request, (now_ms, at_work, timing))?;
                transaction.commit()?;
                Ok(answer)
            });
        match done {
            Ok(answer) => {
                drop(self.answered.send(Ok(answer)));
                Ok(())
            }
            Err(Refused::Named(message)) => {
                drop(self.answered.send(Err(NotDone::Refused(message))));
                Ok(())
            }
            Err(Refused::Store(error)) => {
                drop(self.answered.send(Err(NotDone::Failed)));
                Err(error)
            }
        }
    }
}

/// Answers `request` at `now_ms` in `transaction`, an agent's note in the open task of the only
/// session at work, a claim held by the latest, positions holding as `timing` says.
fn answer(
    transaction: &mut WriteTx<'_>,
    request: &Ask,
    (now_ms, at_work, timing): (i64, &AtWork, &Timing),
) -> Result<ResponseBody, Refused> {
    Ok(match request {
        Ask::Note(note) => ResponseBody::Facts(facts::noted(
            transaction,
            note,
            (now_ms, at_work.sole.as_ref()),
        )?),
        Ask::Facts(ask) => ResponseBody::Facts(facts::answer_ask(transaction, ask, now_ms)?),
        Ask::Claim(at) => ResponseBody::Claims(claims::claimed(
            transaction,
            at,
            (now_ms, at_work.latest.as_ref(), timing),
        )?),
        Ask::Release(at) => {
            ResponseBody::Claims(claims::released(transaction, at, (now_ms, timing))?)
        }
    })
}

#[cfg(test)]
mod tests {
    use cortex_core::ipc::FactsAnswer;
    use cortex_core::{CoordinationConfig, FactKind, FactSource};

    use super::*;
    use crate::test_store::{TestResult, add_file, writer};

    /// A note the writer takes after its asker stopped waiting is dropped unanswered, which tells
    /// the asker it was not taken, and nothing is written; one taken in time is answered, and a
    /// refused one is answered with the refusal and writes nothing.
    #[test]
    fn a_request_taken_too_late_is_never_written() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        add_file(&mut writer, "src/money.py", 1)?;
        let note = Ask::Note(Note {
            text: "Money is kept in cents.".to_owned(),
            kind: FactKind::Convention,
            source: FactSource::User,
            at: vec!["src/money.py".to_owned()],
        });
        let (mut late, answer) = AskJob::new(note.clone());
        late.start_by = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .ok_or("no earlier instant")?;
        let timing = Timing::of(&CoordinationConfig::default());
        late.carry_out(&mut writer, (5, &AtWork::default(), &timing))?;
        assert!(matches!(
            answer.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        let (unread, answer) = AskJob::new(note);
        unread.carry_out(&mut writer, (6, &AtWork::default(), &timing))?;
        assert_eq!(
            answer.try_recv()?,
            Err(NotDone::Refused(
                "src/money.py cannot anchor a fact: Cortex does not read it".to_owned()
            ))
        );
        let (in_time, answer) = AskJob::new(Ask::Facts(FactsAsk::List { all: true }));
        in_time.carry_out(&mut writer, (7, &AtWork::default(), &timing))?;
        assert_eq!(
            answer.try_recv()?,
            Ok(ResponseBody::Facts(FactsAnswer::Listed {
                facts: Vec::new()
            }))
        );
        Ok(())
    }
}
