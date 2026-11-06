//! The reason strict mode gives an agent to go on when it stops: the files that call a signature
//! its task changed and that it has not read or edited since it was warned of them. The daemon
//! asks only when the user turned strict mode on, and gives it once a task.

use std::collections::BTreeSet;

use cortex_core::{RepoPath, Ulid, WarningKind, estimate_tokens};
use cortex_store::{NodeId, Queries, StoreError};

use crate::open_warnings::{plain_path, so_far};

/// The most tokens a stop's reason takes, as estimated: as many as an edit's warnings.
pub const MAX_STOP_REASON_TOKENS: usize = 250;

/// The reason task `task_id` is given to go on when its agent stops, as the store `queries` reads
/// holds the task: none when no call-site warning of it is open.
///
/// # Errors
///
/// The store's errors.
pub fn stop_reason(queries: Queries<'_>, task_id: Ulid) -> Result<Option<String>, StoreError> {
    let so_far = so_far(queries, task_id)?;
    let mut named: BTreeSet<NodeId> = BTreeSet::new();
    let mut files: Vec<RepoPath> = Vec::new();
    for warning in &so_far.open {
        if warning.kind != WarningKind::CallSite || !named.insert(warning.target) {
            continue;
        }
        files.extend(plain_path(queries, warning.target)?);
    }
    Ok(reason_naming(&files))
}

/// The reason naming as many of `files`, the latest warned of first, as fit in
/// [`MAX_STOP_REASON_TOKENS`], with how many more there are; none without files, or when not even
/// the first fits.
fn reason_naming(files: &[RepoPath]) -> Option<String> {
    (1..=files.len()).rev().find_map(|count| {
        let named: Vec<&str> = files.iter().take(count).map(RepoPath::as_str).collect();
        let more = match files.len() - count {
            0 => String::new(),
            more => format!(" and {more} more"),
        };
        let text = format!(
            "[cortex] Strict mode: these files call a signature this task changed and were not \
             read or edited since the warning: {}{more}.\n",
            named.join(", ")
        );
        // INVARIANT: budgeted (a reason past its cap names fewer files, or is not given)
        (estimate_tokens(&text) <= MAX_STOP_REASON_TOKENS).then_some(text)
    })
}
