//! What a task's prompt was given: the files of its work orders whose text reached the agent, each
//! with the lines of the spans the text showed. What the task then did with them earns interest
//! and gives them a part in the task's working set. A held-out task's injection, and a retrieval
//! the gate kept back, gave nothing; nor did the other texts a task is given (a read's line naming
//! the file earlier tasks read next names a file, but did not give it to the prompt).

use cortex_core::{HookKind, LineRange, Ulid};
use cortex_store::{NodeId, Queries, StoreError};

/// One file a task's prompt was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GivenFile {
    /// The file's node.
    pub(crate) node: NodeId,
    /// The lines of each of its spans the text showed, the strongest first.
    pub(crate) spans: Vec<LineRange>,
}

/// The files the work orders of task `task_id` that reached the agent gave, in the order they were
/// given; a span whose lines are no range of lines is left out.
///
/// # Errors
///
/// The store's errors.
pub(crate) fn given_to(queries: Queries<'_>, task_id: Ulid) -> Result<Vec<GivenFile>, StoreError> {
    Ok(queries
        .injections_of_task(task_id)?
        .into_iter()
        .filter(|row| row.record.hook == HookKind::Prompt && row.record.was_delivered())
        .flat_map(|row| row.record.files)
        .map(|file| GivenFile {
            node: file.node,
            spans: file
                .spans
                .iter()
                .filter_map(|(start, end)| LineRange::new(*start, *end).ok())
                .collect(),
        })
        .collect())
}
