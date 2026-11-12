//! The receipts report: what a repository's store holds of what agents' work cost and what Cortex
//! added to it. The arms of the held-out tasks and their comparison once both are large enough,
//! how precise the work orders and warnings Cortex gave were, and the receipt of each session
//! asked for. `cortex receipts` prints it and the map shows it; both read it here, so the two
//! never disagree.
//!
//! A subagent's work is its parent session's: it is counted in the parent's receipt and is never a
//! session of its own here.

use std::collections::BTreeMap;

use cortex_core::{Identifier, Ulid};
use cortex_store::{Queries, SessionRow, StoreError};
use serde::Serialize;

use crate::holdout::{Comparison, compare, weighed};
use crate::pricing::PriceTable;
use crate::receipts::{Activity, Arms, MIN_TASKS_PER_ARM, Precision, SessionReceipt, receipt};

/// Which sessions a report gives receipts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// The session with this id, whenever it started.
    Session(Identifier),
    /// Every session that started at this time or later, in UTC milliseconds.
    StartedSince(i64),
}

/// What the store holds: the arms of the held-out tasks, how precise the work orders given and the
/// warnings were, and the receipts of the sessions asked for.
#[derive(Debug, Clone, Serialize)]
pub struct Recorded {
    /// How many of the repository's tasks were held out and how many were not.
    pub holdout: Holdout,
    /// Of the repository's work orders that reached the agent, how many were labelled and used.
    pub precision: Precision,
    /// Of the files the repository's warnings named, how many were marked at their task's end, and
    /// how many of those were then read or edited.
    pub warnings: Precision,
    /// Each session's receipt, by start.
    pub sessions: Vec<SessionReceipt>,
}

/// The tasks held out from Cortex and the rest, which receipts compare once both are enough.
#[derive(Debug, Clone, Serialize)]
pub struct Holdout {
    /// How many of the repository's tasks each arm holds, whenever they ran.
    #[serde(flatten)]
    pub arms: Arms,
    /// How many each arm needs before they are compared.
    pub needed_per_arm: u64,
    /// The comparison, once each arm has enough tasks that ended.
    pub comparison: Option<Comparison>,
}

/// Why a report could not be made.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// The store could not be read; the operation says what was being read.
    #[error("{operation}: {source}")]
    Store {
        /// What was being read.
        operation: &'static str,
        /// The store's error.
        source: StoreError,
    },
    /// The session asked for is not in the store.
    #[error("no session {0} in the store")]
    NoSession(String),
    /// The session asked for is a subagent, whose work its parent's receipt counts.
    #[error("session {session} is a subagent of session {parent}, whose receipts count its work")]
    Subagent {
        /// The session asked for.
        session: String,
        /// Its parent.
        parent: String,
    },
}

/// `result` with a store error told as `operation`.
fn reading<T>(operation: &'static str, result: Result<T, StoreError>) -> Result<T, ReportError> {
    result.map_err(|source| ReportError::Store { operation, source })
}

/// The report of the store `queries` read, for the sessions `selection` names, priced from
/// `prices` when there is a table.
///
/// # Errors
///
/// A session asked for that is not stored or is a subagent, or a store that cannot be read.
pub fn recorded(
    queries: Queries<'_>,
    selection: &Selection,
    prices: Option<&PriceTable>,
) -> Result<Recorded, ReportError> {
    let (held_out, treated) = reading("counting the held-out tasks", queries.holdout_arms())?;
    let (labelled, used) = reading(
        "counting the labelled work orders given",
        queries.delivered_precision(),
    )?;
    let warned = reading("counting the warnings given", queries.warning_counts())?;
    let arms = Arms { held_out, treated };
    let comparison = if arms.is_enough() {
        compared(queries)?
    } else {
        None
    };
    let sessions = sessions(queries, selection)?
        .into_iter()
        .map(|session| Ok(receipt(&activity(queries, session)?, prices)))
        .collect::<Result<_, ReportError>>()?;
    Ok(Recorded {
        holdout: Holdout {
            arms,
            needed_per_arm: MIN_TASKS_PER_ARM,
            comparison,
        },
        precision: Precision { labelled, used },
        warnings: Precision {
            labelled: warned.labelled,
            used: warned.resolved,
        },
        sessions,
    })
}

/// The holdout comparison over every session the store holds, a subagent's work counted in its
/// parent's, each task's usage placed as its receipt places it; none until each arm has enough
/// tasks that ended.
fn compared(queries: Queries<'_>) -> Result<Option<Comparison>, ReportError> {
    let sessions = reading(
        "reading the sessions to compare",
        queries.sessions_started_since(i64::MIN),
    )?;
    let receipts: Vec<SessionReceipt> = sessions
        .into_iter()
        .filter(|session| session.record.parent_session_id.is_none())
        .map(|session| Ok(receipt(&activity(queries, session)?, None)))
        .collect::<Result<_, ReportError>>()?;
    let (treated, held_out) = weighed(&receipts);
    Ok(compare(&treated, &held_out))
}

/// The sessions `selection` names: the one it names, or each that started since the time it
/// gives, never a subagent.
fn sessions(queries: Queries<'_>, selection: &Selection) -> Result<Vec<SessionRow>, ReportError> {
    let id = match selection {
        Selection::StartedSince(since_ms) => {
            let started = reading(
                "reading the sessions",
                queries.sessions_started_since(*since_ms),
            )?;
            return Ok(started
                .into_iter()
                .filter(|session| session.record.parent_session_id.is_none())
                .collect());
        }
        Selection::Session(id) => id,
    };
    let session = reading("reading the session", queries.session(id))?
        .ok_or_else(|| ReportError::NoSession(id.as_str().to_owned()))?;
    if let Some(parent) = &session.record.parent_session_id {
        return Err(ReportError::Subagent {
            session: id.as_str().to_owned(),
            parent: parent.as_str().to_owned(),
        });
    }
    Ok(vec![session])
}

/// What `session` did: its tasks, its responses and its subagents', each task's tool calls, the
/// calls it and its subagents made while it had no task, their compactions, and what Cortex gave
/// it.
fn activity(queries: Queries<'_>, session: SessionRow) -> Result<Activity, ReportError> {
    const READING: &str = "reading the activity of a session";
    let id = &session.record.session_id;
    let tasks = reading(READING, queries.tasks_of_session(id))?;
    let subagents = reading(READING, queries.sessions_started_by(id))?;
    let mut usage = reading(READING, queries.usage_of_session(id))?;
    let mut outside_tool_calls = reading(READING, queries.tool_calls_outside_tasks(id))?;
    let mut compactions_at_ms = BTreeMap::new();
    compactions_at_ms.insert(
        id.clone(),
        reading(READING, queries.compactions_of_session(id))?,
    );
    for subagent in &subagents {
        let subagent = &subagent.record.session_id;
        usage.extend(reading(READING, queries.usage_of_session(subagent))?);
        let calls = reading(READING, queries.tool_calls_outside_tasks(subagent))?;
        outside_tool_calls = outside_tool_calls.saturating_add(calls);
        let compacted = reading(READING, queries.compactions_of_session(subagent))?;
        compactions_at_ms.insert(subagent.clone(), compacted);
    }
    let task_ids: Vec<Ulid> = tasks.iter().map(|task| task.record.task_id).collect();
    let tool_calls = reading(READING, queries.tool_calls_of_tasks(&task_ids))?;
    let injections = reading(READING, queries.injections_of_session(id))?
        .into_iter()
        .map(|row| row.record)
        .collect();
    Ok(Activity {
        subagents: u64::try_from(subagents.len()).unwrap_or(u64::MAX),
        session,
        tasks,
        usage,
        tool_calls,
        outside_tool_calls,
        compactions_at_ms,
        injections,
    })
}

#[cfg(test)]
mod tests {
    use cortex_core::{Agent, RepoId, TaskOutcome};
    use cortex_store::{Reader, SessionRecord, TaskRecord, UsageRow, Writer, WriterOptions};

    use super::*;

    /// Any failure fails the test.
    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A store holding one session of Claude Code with `per_arm` held-out and `per_arm` treated
    /// tasks, each ended with success and one response of 100 fresh input tokens.
    fn store_with(per_arm: u32) -> TestResult<tempfile::TempDir> {
        let root = tempfile::tempdir()?;
        let mut writer = Writer::open(&WriterOptions {
            folder: &root.path().join(".cortex"),
            repo_id: &RepoId::of(root.path()),
            mmap_bytes: 0,
            now_ms: 1,
        })?;
        let session = Identifier::new("s")?;
        let mut transaction = writer.transaction()?;
        transaction.record_session(&SessionRecord {
            session_id: session.clone(),
            agent: Agent::Claude,
            agent_version: None,
            model: None,
            parent_session_id: None,
            cwd: None,
            transcript_path: None,
            started_at_ms: 1_000,
        })?;
        let mut responses = Vec::new();
        for at in 0..per_arm * 2 {
            let task_id = Ulid::from_seed(u64::from(at), b"arms");
            let started_at_ms = 10_000 + i64::from(at) * 10_000;
            transaction.start_task(&TaskRecord {
                task_id,
                session_id: session.clone(),
                started_at_ms,
                prompt_hash: None,
                prompt_excerpt: None,
                holdout: Some(at % 2 == 1),
            })?;
            transaction.end_task(task_id, started_at_ms + 5_000, TaskOutcome::Success)?;
            responses.push(UsageRow {
                session_id: session.clone(),
                turn: at + 1,
                at_ms: started_at_ms + 1_000,
                model: None,
                input: 100,
                cache_write: 0,
                cache_write_1h: None,
                cache_read: 0,
                output: 1,
                reasoning: None,
                service_tier: None,
                speed: None,
                inference_geo: None,
                response: None,
            });
        }
        transaction.replace_usage(&responses)?;
        transaction.commit()?;
        Ok(root)
    }

    /// The report of the store at `root` for every session.
    fn report_of(root: &tempfile::TempDir) -> TestResult<Recorded> {
        let reader = Reader::open(&root.path().join(".cortex"), 0)?;
        Ok(recorded(
            reader.queries(),
            &Selection::StartedSince(0),
            None,
        )?)
    }

    /// With 29 ended tasks a side the arms are counted and not compared; with 30 a side, the
    /// least the comparison needs, they are compared over 30 and 30.
    #[test]
    fn the_arms_are_compared_from_thirty_tasks_a_side() -> TestResult {
        let few = report_of(&store_with(29)?)?;
        assert_eq!(
            few.holdout.arms,
            Arms {
                held_out: 29,
                treated: 29
            }
        );
        assert_eq!(few.holdout.needed_per_arm, 30);
        assert!(few.holdout.comparison.is_none());
        let enough = report_of(&store_with(30)?)?;
        assert_eq!(
            enough.holdout.arms,
            Arms {
                held_out: 30,
                treated: 30
            }
        );
        let comparison = enough
            .holdout
            .comparison
            .ok_or("no comparison at 30 a side")?;
        assert_eq!(comparison.tasks, [30, 30]);
        assert_eq!(enough.sessions.len(), 1);
        Ok(())
    }

    /// A session asked for that is not stored is refused by its id.
    #[test]
    fn a_session_not_stored_is_refused() -> TestResult {
        let root = store_with(1)?;
        let reader = Reader::open(&root.path().join(".cortex"), 0)?;
        let selection = Selection::Session(Identifier::new("nobody")?);
        let refused = recorded(reader.queries(), &selection, None);
        assert!(
            matches!(&refused, Err(ReportError::NoSession(id)) if id == "nobody"),
            "{refused:?}"
        );
        Ok(())
    }
}
