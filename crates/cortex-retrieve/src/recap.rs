//! The recap a session gets after its context is compacted: what its open task was doing, so that
//! the agent goes on without finding it again. It is the task's working set (see
//! [`crate::working_set`]: the files it edited, the files it was warned of and has not read or
//! edited since, the project facts it was given that still hold, its prompt) under its own header,
//! held to [`MAX_RECAP_TOKENS`].

use std::path::Path;

use cortex_core::estimate_tokens;
use cortex_store::{Queries, StoreError, TaskRow};

use crate::working_set::{Frame, working_set};

/// The most tokens a recap takes, as estimated.
pub const MAX_RECAP_TOKENS: usize = 500;

/// The first line of every recap.
const HEADER: &str = "[cortex] Before the compaction, the task in progress:\n";

/// The recap a session gets after a compaction of its open task `task`, as the store `queries`
/// reads holds it and the repository at `root` holds its files, each read up to `max_bytes`;
/// none when the task has nothing to recall.
///
/// # Errors
///
/// The store's errors.
pub fn recap(
    queries: Queries<'_>,
    task: &TaskRow,
    (root, max_bytes): (&Path, u64),
) -> Result<Option<String>, StoreError> {
    let frame = Frame {
        header: HEADER,
        fits: |text| estimate_tokens(text) <= MAX_RECAP_TOKENS,
    };
    working_set(queries, task, (root, max_bytes), frame)
}
