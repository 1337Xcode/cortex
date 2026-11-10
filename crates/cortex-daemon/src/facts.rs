//! The writer's answers to fact requests from `cortex note` and `cortex facts`: a note's anchors
//! found from the paths and symbol names it gives, the note stated as the user's; the list of
//! facts with their anchors named as a note names them; and the user's word on one fact (confirm,
//! retire, correct). Each request is answered in one transaction, refused in words the user reads
//! when it names something wrongly.

use cortex_core::ipc::{FactLine, FactsAnswer, FactsAsk, GONE_ANCHOR, Note, NoteOutcome};
use cortex_core::{FactSource, FactStatus, Identifier, RepoPath};
use cortex_learn::{
    Asserted, Assertion, FactError, assert_fact, confirm_fact, edit_fact, retire_fact,
};
use cortex_store::{FactId, FactRow, NodeId, Queries, StoreError, WriteTx};

use crate::asked::Refused;

/// Carries out the user's word `ask` on the facts at `now_ms` in `transaction`.
pub(crate) fn answer_ask(
    transaction: &mut WriteTx<'_>,
    ask: &FactsAsk,
    now_ms: i64,
) -> Result<FactsAnswer, Refused> {
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

/// States `note` at `now_ms` in `transaction`, as its source says: the user's, an import's (a
/// teammate's fact, from `cortex import`), or, in the open task of `at_work`, an agent's.
pub(crate) fn noted(
    transaction: &mut WriteTx<'_>,
    note: &Note,
    (now_ms, at_work): (i64, Option<&Identifier>),
) -> Result<FactsAnswer, Refused> {
    let nodes = note
        .at
        .iter()
        .map(|anchor| anchor_of(transaction.queries(), anchor))
        .collect::<Result<Vec<_>, _>>()?;
    // An import keeps its source: a teammate's fact is a candidate until a person here confirms
    // it, never the user's own.
    let source = note.source;
    let task = match (source, at_work) {
        (FactSource::Agent, Some(session)) => transaction
            .queries()
            .open_task(session)?
            .map(|task| task.record.task_id),
        _ => None,
    };
    let assertion = Assertion {
        text: &note.text,
        kind: note.kind,
        source,
        task,
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
    let named = queries.symbols_in_file_called(file, symbol)?;
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
                        .unwrap_or_else(|| GONE_ANCHOR.to_owned())
                })
                .collect(),
            text: row.record.text,
            uses: row.uses,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use cortex_coord::Timing;
    use cortex_core::ipc::ResponseBody;
    use cortex_core::{Agent, ContentHash, CoordinationConfig, FactKind, FileFlags, Ulid};
    use cortex_store::{FileIndex, FileRecord, SessionRecord, TaskRecord, Writer};

    use super::*;
    use crate::asked::{Ask, AskJob, AtWork};
    use crate::test_store::{TestResult, writer};

    /// Stores `src/money.py`, read, holding "cents", and session `s1` with its open task, made at 1;
    /// returns the task.
    fn money_and_a_task(writer: &mut Writer) -> TestResult<Ulid> {
        let task = Ulid::from_seed(1, b"task");
        let session = Identifier::new("s1")?;
        let mut transaction = writer.transaction()?;
        transaction.replace_file(&FileIndex {
            file: FileRecord {
                path: RepoPath::new("src/money.py")?,
                language: None,
                size_bytes: 5,
                line_count: Some(1),
                content_hash: Some(ContentHash::of(b"cents")),
                mtime_ns: 1,
                flags: FileFlags::empty(),
                indexed_at_ms: 1,
            },
            symbols: Vec::new(),
            refs: Vec::new(),
            passes_on: Vec::new(),
        })?;
        transaction.record_session(&SessionRecord {
            session_id: session.clone(),
            agent: Agent::Claude,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 1,
        })?;
        transaction.start_task(&TaskRecord {
            task_id: task,
            session_id: session,
            started_at_ms: 1,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: Some(false),
        })?;
        transaction.commit()?;
        Ok(task)
    }

    /// An agent's note is a candidate, asserted in the open task of the session at work, `s1`, when
    /// it is the only one at work; with no session at work, or with `s1` the latest of two, in no
    /// task: the note cannot be told to be `s1`'s. A user's note is active, in no task, whoever is at
    /// work. An import's is a candidate, in no task, whoever is at work, and its source stays an
    /// import.
    #[test]
    fn an_agent_s_note_is_a_candidate_in_the_task_at_work() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut writer = writer(root.path())?;
        let task = money_and_a_task(&mut writer)?;
        let at_work = Identifier::new("s1")?;
        let note = |text: &str, source| {
            Ask::Note(Note {
                text: text.to_owned(),
                kind: FactKind::Convention,
                source,
                at: vec!["src/money.py".to_owned()],
            })
        };
        let alone = AtWork {
            latest: Some(at_work.clone()),
            sole: Some(at_work.clone()),
        };
        let of_two = AtWork {
            latest: Some(at_work),
            sole: None,
        };
        let cases = [
            (note("Money is kept in cents.", FactSource::Agent), &alone),
            (
                note("Money never floats.", FactSource::Agent),
                &AtWork::default(),
            ),
            (note("Money is exact.", FactSource::User), &alone),
            (note("Money is never rounded.", FactSource::Import), &alone),
            (note("Money has two decimals.", FactSource::Agent), &of_two),
        ];
        for (request, at_work) in cases {
            let (job, answer) = AskJob::new(request);
            let timing = Timing::of(&CoordinationConfig::default());
            job.carry_out(&mut writer, (5, at_work, &timing))?;
            assert!(matches!(
                answer.try_recv()?,
                Ok(ResponseBody::Facts(FactsAnswer::Noted { .. }))
            ));
        }
        let facts: Vec<(String, FactStatus, Option<Ulid>)> = [1, 2, 3, 4, 5]
            .into_iter()
            .map(|number| {
                let id = writer.queries().fact_id(number)?.ok_or("a fact")?;
                let row = writer.queries().fact(id)?.ok_or("a fact")?;
                Ok((row.record.text, row.record.status, row.record.task))
            })
            .collect::<TestResult<_>>()?;
        assert_eq!(
            facts,
            [
                (
                    "Money is kept in cents.".to_owned(),
                    FactStatus::Candidate,
                    Some(task)
                ),
                (
                    "Money never floats.".to_owned(),
                    FactStatus::Candidate,
                    None
                ),
                ("Money is exact.".to_owned(), FactStatus::Active, None),
                (
                    "Money is never rounded.".to_owned(),
                    FactStatus::Candidate,
                    None
                ),
                (
                    "Money has two decimals.".to_owned(),
                    FactStatus::Candidate,
                    None
                ),
            ]
        );
        let imported = writer.queries().fact_id(4)?.ok_or("a fact")?;
        let row = writer.queries().fact(imported)?.ok_or("a fact")?;
        assert_eq!(row.record.source, FactSource::Import);
        Ok(())
    }
}
