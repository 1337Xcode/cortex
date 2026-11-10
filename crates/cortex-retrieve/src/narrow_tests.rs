//! The narrow tests of a task: the test files of the files it edited, for the hint a whole-suite
//! run gets. A file's own tests come first (named for it: `tests/test_ops.py` for `calc/ops.py`),
//! then the test files that import it, the strongest tie first; a test file the task edited
//! counts for itself. At most [`MAX_NARROW_TESTS`], each once, each a plain path.
//!
//! ```text
//! [cortex] Tests for the files changed in this task: tests/test_ops.py, tests/test_main.py
//! (`pytest tests/test_ops.py tests/test_main.py`).
//! ```

use std::fmt::Write as _;

use cortex_core::{EventKind, RepoPath, Ulid, estimate_tokens, is_plain};
use cortex_store::{NodeId, Queries, StoreError};

use crate::warnings::is_own_test;

/// The most test files the hint names.
pub const MAX_NARROW_TESTS: usize = 3;

/// The most tokens the hint takes, as estimated.
pub const MAX_NARROW_HINT_TOKENS: usize = 80;

/// How many of a task's latest events are read for the files it edited.
const EVENTS_READ: usize = 2_000;

/// A test file of the task's edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrowTest {
    /// The test file.
    pub path: RepoPath,
    /// Its node.
    pub node: NodeId,
}

/// The test files of what task `task` edited, the best first, at most [`MAX_NARROW_TESTS`]: none
/// when it edited nothing with tests.
///
/// # Errors
///
/// The store's errors.
pub fn narrow_tests(queries: Queries<'_>, task: Ulid) -> Result<Vec<NarrowTest>, StoreError> {
    let mut edited: Vec<NodeId> = Vec::new();
    for row in queries.events_of_task(task, EVENTS_READ)? {
        let event = &row.record;
        let is_done = event.meta.is_success != Some(false);
        if let (EventKind::Edit, Some(node), true) = (event.kind, event.node_id, is_done)
            && !edited.contains(&node)
        {
            edited.push(node);
        }
    }
    // Each candidate: (is the edited file's own test, tie, path, node).
    let mut found: Vec<(bool, f64, RepoPath, NodeId)> = Vec::new();
    for node in &edited {
        let Some((_, path)) = queries.node_file(*node)? else {
            continue;
        };
        if is_test_path(path.as_str()) {
            found.push((true, 1.0, path.clone(), *node));
        }
        for (test, weight) in queries.test_files_into(*node)? {
            let Some((_, test_path)) = queries.node_file(test)? else {
                continue;
            };
            let is_own = is_own_test(test_path.as_str(), path.as_str());
            found.push((is_own, weight, test_path, test));
        }
    }
    found.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.total_cmp(&left.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    let mut tests: Vec<NarrowTest> = Vec::new();
    for (_, _, path, node) in found {
        if tests.len() == MAX_NARROW_TESTS {
            break;
        }
        if is_plain(path.as_str()) && !tests.iter().any(|test| test.node == node) {
            tests.push(NarrowTest { path, node });
        }
    }
    Ok(tests)
}

/// The hint naming `tests`, with `command`, the narrow command that runs them, when one can be
/// written, and how many of `tests`, the first, it names: none when there are no tests, or the
/// hint does not fit its cap even with one file.
#[must_use]
pub fn narrow_hint(tests: &[NarrowTest], command: Option<&str>) -> Option<(String, usize)> {
    let mut named = tests.len();
    while named > 0 {
        let paths: Vec<&str> = tests
            .iter()
            .take(named)
            .map(|test| test.path.as_str())
            .collect();
        let mut text = format!(
            "[cortex] Tests for the files changed in this task: {}",
            paths.join(", ")
        );
        match command.filter(|command| is_plain(command) && !command.contains('`')) {
            Some(command) if named == tests.len() => {
                let _ = writeln!(text, " (`{command}`).");
            }
            _ => text.push_str(".\n"),
        }
        // INVARIANT: budgeted (fewer files are named until the hint fits its cap)
        if estimate_tokens(&text) <= MAX_NARROW_HINT_TOKENS {
            return Some((text, named));
        }
        named -= 1;
    }
    None
}

/// Whether `path` names a test file by the names test runners look for: under a `tests` or `test`
/// folder, or named `test_*`, `*_test.*`, `*.test.*` or `*.spec.*`.
fn is_test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.split('/')
        .any(|part| part == "tests" || part == "test")
        || name.starts_with("test_")
        || name.contains("_test.")
        || name.contains(".test.")
        || name.contains(".spec.")
}
