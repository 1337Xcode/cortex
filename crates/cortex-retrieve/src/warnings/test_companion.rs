//! The test companion: the edited file has tests the task has not touched. Told once a task, on
//! the first edit of a file that has them.

use cortex_core::{WarningKind, WarningsConfig};

use super::{EditFacts, TaskSoFar, Warning};
use crate::plain::is_plain;

/// The most test files one warning names.
pub const MAX_LISTED_TESTS: usize = 3;

/// The warning naming the edited file's tests the task has not read or edited, unless the task was
/// warned about tests already.
pub(super) fn warnings(facts: &EditFacts, task: &TaskSoFar, _: &WarningsConfig) -> Vec<Warning> {
    let was_warned = task
        .warned
        .iter()
        .any(|(kind, _, _)| *kind == WarningKind::Test);
    if was_warned || !is_plain(facts.edited.path.as_str()) {
        return Vec::new();
    }
    let edited = facts.edited.node;
    let open: Vec<_> = facts
        .tests
        .iter()
        .filter(|test| {
            test.file.node != edited
                && !task.touched.contains(&test.file.node)
                && is_plain(test.file.path.as_str())
        })
        .take(MAX_LISTED_TESTS)
        .collect();
    let Some(strength) = open
        .iter()
        .map(|test| test.confidence)
        .max_by(f64::total_cmp)
    else {
        return Vec::new();
    };
    let paths: Vec<&str> = open.iter().map(|test| test.file.path.as_str()).collect();
    vec![Warning {
        kind: WarningKind::Test,
        source: edited,
        targets: open.iter().map(|test| test.file.node).collect(),
        strength,
        line: format!("Tests for {}: {}", facts.edited.path, paths.join(", ")),
    }]
}
