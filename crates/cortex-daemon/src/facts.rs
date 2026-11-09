//! The writer's answers to fact requests from `cortex note` and `cortex facts`: a note's anchors
//! found from the paths and symbol names it gives, the note stated as the user's; the list of
//! facts with their anchors named as a note names them; and the user's word on one fact (confirm,
//! retire, correct). Each request is answered in one transaction, refused in words the user reads
//! when it names something wrongly.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use cortex_core::ipc::{FactLine, FactsAnswer, FactsAsk, Note, NoteOutcome};
use cortex_core::{FactSource, FactStatus, RepoPath};
use cortex_learn::{
    Asserted, Assertion, FactError, assert_fact, confirm_fact, edit_fact, retire_fact,
};
use cortex_store::{FactId, FactRow, NodeId, Queries, StoreError, WriteTx, Writer};

/// How long a fact request may wait for the writer before it is dropped unwritten: long enough for
/// a batch of the index to end, short enough that the person who typed it is told.
pub(crate) const FACTS_START_WITHIN: Duration = Duration::from_secs(10);

/// How long the writer has to answer a fact request it took: one transaction of a few rows.
pub(crate) const FACTS_WRITE_WITHIN: Duration = Duration::from_secs(5);

/// A fact request, as the writer takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FactsRequest {
    /// A fact the user notes.
    Note(Note),
    /// The user's word on the facts.
    Ask(FactsAsk),
}

/// A fact request for the writer, with where to answer it and until when to start on it.
#[derive(Debug)]
pub(crate) struct FactsJob {
    /// What is asked.
    request: FactsRequest,
    /// When the asker stops waiting for the writer to start: a job taken later is dropped
    /// unwritten, so that a note the asker was told failed is never written after all.
    start_by: Instant,
    /// Where the writer answers.
    answered: SyncSender<Result<FactsAnswer, String>>,
}

impl FactsJob {
    /// A job for `request`, to be started within [`FACTS_START_WITHIN`], and where its answer
    /// arrives.
    pub(crate) fn new(request: FactsRequest) -> (Self, Receiver<Result<FactsAnswer, String>>) {
        let (answered, answer) = mpsc::sync_channel(1);
        let job = Self {
            request,
            start_by: Instant::now() + FACTS_START_WITHIN,
            answered,
        };
        (job, answer)
    }

    /// Carries out the job at `now_ms` with `writer`, in a transaction of its own, and answers it
    /// only once that transaction is committed: what the asker is told was done is written. A
    /// refusal rolls the transaction back and is the answer; a job that waited too long is dropped
    /// unanswered and nothing is written.
    ///
    /// # Errors
    ///
    /// The store's errors, the commit's among them, which are also the job's answer.
    pub(crate) fn carry_out(self, writer: &mut Writer, now_ms: i64) -> Result<(), StoreError> {
        if Instant::now() > self.start_by {
            return Ok(());
        }
        let done = writer
            .transaction()
            .map_err(Refused::from)
            .and_then(|mut transaction| {
                let answer = answer(&mut transaction, &self.request, now_ms)?;
                transaction.commit()?;
                Ok(answer)
            });
        match done {
            Ok(answer) => {
                drop(self.answered.send(Ok(answer)));
                Ok(())
            }
            Err(Refused::Named(message)) => {
                drop(self.answered.send(Err(message)));
                Ok(())
            }
            Err(Refused::Store(error)) => {
                drop(self.answered.send(Err(error.to_string())));
                Err(error)
            }
        }
    }
}

/// Why a fact request was refused.
#[derive(Debug)]
enum Refused {
    /// The request named something wrongly, in words the user reads.
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

/// Answers `request` at `now_ms` in `transaction`.
fn answer(
    transaction: &mut WriteTx<'_>,
    request: &FactsRequest,
    now_ms: i64,
) -> Result<FactsAnswer, Refused> {
    let ask = match request {
        FactsRequest::Note(note) => return noted(transaction, note, now_ms),
        FactsRequest::Ask(ask) => ask,
    };
    let done = match ask {
        FactsAsk::List { all } => {
            return Ok(FactsAnswer::Listed {
                facts: listed(transaction.queries(), *all)?,
            });
        }
        FactsAsk::Confirm { id } => {
            let fact = numbered(transaction.queries(), *id)?;
            confirm_fact(transaction, fact, now_ms).map(|()| FactsAnswer::Confirmed { id: *id })
        }
        FactsAsk::Retire { id } => {
            let fact = numbered(transaction.queries(), *id)?;
            retire_fact(transaction, fact, now_ms).map(|()| FactsAnswer::Retired { id: *id })
        }
        FactsAsk::Edit { id, text } => {
            let fact = numbered(transaction.queries(), *id)?;
            edit_fact(transaction, fact, text, now_ms).map(|by| FactsAnswer::Edited {
                id: *id,
                by: by.get(),
            })
        }
    };
    done.map_err(|error| refused(transaction.queries(), error))
}

/// The fact numbered `id`, as a person typed it.
fn numbered(queries: Queries<'_>, id: i64) -> Result<FactId, Refused> {
    queries
        .fact_id(id)?
        .ok_or_else(|| Refused::Named(format!("there is no fact {id}")))
}

/// States `note` as the user's at `now_ms` in `transaction`.
fn noted(transaction: &mut WriteTx<'_>, note: &Note, now_ms: i64) -> Result<FactsAnswer, Refused> {
    let nodes = note
        .at
        .iter()
        .map(|anchor| anchor_of(transaction.queries(), anchor))
        .collect::<Result<Vec<_>, _>>()?;
    let assertion = Assertion {
        text: &note.text,
        kind: note.kind,
        source: FactSource::User,
        task: None,
        nodes: &nodes,
    };
    let (id, outcome) = match assert_fact(transaction, &assertion, now_ms) {
        Ok(Asserted::Added(id)) => (id, NoteOutcome::Added),
        Ok(Asserted::Merged(id)) => (id, NoteOutcome::Merged),
        Ok(Asserted::Activated(id)) => (id, NoteOutcome::Activated),
        Err(error) => return Err(refused(transaction.queries(), error)),
    };
    Ok(FactsAnswer::Noted {
        id: id.get(),
        outcome,
    })
}

/// `error` in words the user reads, an anchor named by its path; the store's failures stay
/// the store's.
fn refused(queries: Queries<'_>, error: FactError) -> Refused {
    match error {
        FactError::Store(StoreError::FactTooLong { chars }) => Refused::Named(format!(
            "a fact holds at most 240 characters; this one holds {chars}"
        )),
        FactError::Store(StoreError::FactUnanchored) => {
            Refused::Named("a fact needs a file or a symbol it is about".to_owned())
        }
        FactError::Store(error @ (StoreError::FactBlank | StoreError::FactNotPlain)) => {
            Refused::Named(error.to_string())
        }
        FactError::Store(error) => Refused::Store(error),
        FactError::Unanchorable { node } => match queries.anchor_names(&[node]) {
            Err(error) => Refused::Store(error),
            Ok(names) => Refused::Named(match names.into_values().next() {
                Some(name) => format!("{name} cannot anchor a fact: Cortex does not read it"),
                None => "a file or symbol the fact is about is gone: retire the fact, or note it \
                         again where the code is now"
                    .to_owned(),
            }),
        },
        other => Refused::Named(other.to_string()),
    }
}

/// The node `anchor` names: the repository's root for `.`, a stored file at that repository path,
/// or a symbol in one, given after
/// the last `:`, by its qualified name, or by its name when no symbol has that qualified name and
/// one alone has that name. A path that is a stored file whole is that file, so a file whose name
/// holds `:` is found too.
fn anchor_of(queries: Queries<'_>, anchor: &str) -> Result<NodeId, Refused> {
    if anchor == "." {
        return queries.folder_node_at(&RepoPath::root())?.ok_or_else(|| {
            Refused::Named("the repository is not indexed yet: try again once it is".to_owned())
        });
    }
    if let Some(node) = file_node(queries, anchor)? {
        return Ok(node);
    }
    let not_indexed = |path: &str| Refused::Named(format!("{path} is not a file Cortex indexes"));
    let Some((path_text, symbol)) = anchor.rsplit_once(':') else {
        return Err(not_indexed(anchor));
    };
    let Some((file, path)) = file_node(queries, path_text)?
        .map(|node| queries.node_file(node))
        .transpose()?
        .flatten()
    else {
        return Err(not_indexed(path_text));
    };
    let symbols = queries.symbols_in_file(file)?;
    let by_qualified: Vec<_> = symbols
        .iter()
        .filter(|row| row.qual_name.as_str() == symbol)
        .collect();
    let named = if by_qualified.is_empty() {
        symbols
            .iter()
            .filter(|row| row.name.as_str() == symbol)
            .collect()
    } else {
        by_qualified
    };
    let path = path.as_str();
    match named.as_slice() {
        [only] => queries
            .symbol_node(only.id)?
            .ok_or_else(|| Refused::Named(format!("{path} defines no symbol {symbol}"))),
        [] => Err(Refused::Named(format!("{path} defines no symbol {symbol}"))),
        many => Err(Refused::Named(format!(
            "{path} defines {} symbols named {symbol}: give its qualified name",
            many.len()
        ))),
    }
}

/// The node of the stored file at `text`, when `text` is a repository path and a file is stored
/// there.
fn file_node(queries: Queries<'_>, text: &str) -> Result<Option<NodeId>, StoreError> {
    match RepoPath::new(text) {
        Ok(path) if !path.is_root() => queries.file_node_at(&path),
        _ => Ok(None),
    }
}

/// The current facts, oldest first: active, candidate and suspect ones, and retired ones too when
/// `all` is set, each with its anchors named as a note names them, a gone one as `(gone)`.
fn listed(queries: Queries<'_>, all: bool) -> Result<Vec<FactLine>, StoreError> {
    let mut statuses = vec![
        FactStatus::Active,
        FactStatus::Candidate,
        FactStatus::Suspect,
    ];
    if all {
        statuses.push(FactStatus::Retired);
    }
    let mut rows: Vec<FactRow> = Vec::new();
    for status in statuses {
        rows.extend(queries.current_facts(status)?);
    }
    rows.sort_by_key(|row| row.id);
    let nodes: Vec<NodeId> = rows
        .iter()
        .flat_map(|row| row.record.anchors.iter().map(|anchor| anchor.node))
        .collect();
    let names = queries.anchor_names(&nodes)?;
    Ok(rows
        .into_iter()
        .map(|row| FactLine {
            id: row.id.get(),
            status: row.record.status,
            kind: row.record.kind,
            anchors: row
                .record
                .anchors
                .iter()
                .map(|anchor| {
                    names
                        .get(&anchor.node)
                        .cloned()
                        .unwrap_or_else(|| "(gone)".to_owned())
                })
                .collect(),
            text: row.record.text,
            uses: row.uses,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use cortex_core::FactKind;

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
        let note = FactsRequest::Note(Note {
            text: "Money is kept in cents.".to_owned(),
            kind: FactKind::Convention,
            at: vec!["src/money.py".to_owned()],
        });
        let (mut late, answer) = FactsJob::new(note.clone());
        late.start_by = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .ok_or("no earlier instant")?;
        late.carry_out(&mut writer, 5)?;
        assert!(matches!(
            answer.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        let (unread, answer) = FactsJob::new(note);
        unread.carry_out(&mut writer, 6)?;
        assert_eq!(
            answer.try_recv()?,
            Err("src/money.py cannot anchor a fact: Cortex does not read it".to_owned())
        );
        let (in_time, answer) = FactsJob::new(FactsRequest::Ask(FactsAsk::List { all: true }));
        in_time.carry_out(&mut writer, 7)?;
        assert_eq!(
            answer.try_recv()?,
            Ok(FactsAnswer::Listed { facts: Vec::new() })
        );
        Ok(())
    }
}
