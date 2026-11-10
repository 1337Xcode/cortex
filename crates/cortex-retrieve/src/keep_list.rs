//! The keep-list a session's agent is given just before its context is compacted: the open task's
//! working set (see [`crate::working_set`]: the files it edited, the files it was warned of and
//! has not read or edited since, the project facts it was given that still hold, its prompt) as
//! items for the compaction's summary to keep, so that the summary carries them and the agent goes
//! on without finding them again. The same lines the recap after the compaction gives, under a
//! header that asks the summary to keep them, held to [`MAX_KEEP_LIST_CHARS`].

use std::path::Path;

use cortex_store::{Queries, StoreError, TaskRow};

use crate::working_set::{Frame, working_set};

/// The most characters a keep-list takes, its header and line breaks included, counted in UTF-16
/// code units as Claude Code counts a hook's output: never fewer than its characters.
pub const MAX_KEEP_LIST_CHARS: usize = 600;

/// The first line of every keep-list; each line after it says what the task did.
const HEADER: &str = "[cortex] Keep in the summary that the task in progress:\n";

/// The keep-list for a compaction of the open task `task`, as the store `queries` reads holds it
/// and the repository at `root` holds its files, each read up to `max_bytes`; none when the task
/// has nothing to keep.
///
/// # Errors
///
/// The store's errors.
pub fn keep_list(
    queries: Queries<'_>,
    task: &TaskRow,
    (root, max_bytes): (&Path, u64),
) -> Result<Option<String>, StoreError> {
    let frame = Frame {
        header: HEADER,
        fits: |text| text.encode_utf16().count() <= MAX_KEEP_LIST_CHARS,
    };
    working_set(queries, task, (root, max_bytes), frame)
}
