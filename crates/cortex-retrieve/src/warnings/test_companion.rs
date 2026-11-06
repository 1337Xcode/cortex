//! The test companion: the edited file has tests of its own the task has not touched. Told once a
//! task, on the first edit of a file that has them.
//!
//! A file's own tests are the test files named after it (`test_orders.py`, `orders_test.go`,
//! `orders.test.ts`, `OrdersTest.java` for `orders`). A test file that only imports the edited one
//! tests something else through it, and is not named: on past commits such files were almost never
//! changed with the file they import.

use cortex_core::{WarningKind, WarningsConfig};

use super::{EditFacts, TaskSoFar, Warning};
use crate::plain::is_plain;

/// The most test files one warning names.
pub const MAX_LISTED_TESTS: usize = 3;

/// The prefixes a test file's name may add to the name of the file it tests.
const TEST_PREFIXES: [&str; 3] = ["test_", "test-", "Test"];

/// The suffixes a test file's name may add to the name of the file it tests.
const TEST_SUFFIXES: [&str; 7] = ["_test", "-test", "_spec", "-spec", "Tests", "Test", "Spec"];

/// The warning naming the edited file's own tests the task has not read or edited, unless the task
/// was warned about tests already.
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
                && is_own_test(test.file.path.as_str(), facts.edited.path.as_str())
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

/// Whether the file at `test` is named as the tests of the file at `edited`: its name up to the
/// first dot, less a test prefix or suffix, is the edited file's name up to the first dot, letter
/// case aside. `tests/test_orders.py`, `orders_test.go`, `src/orders.test.ts`, `OrdersTest.java`
/// and `tests/orders.rs` are the tests of `src/orders.py` (or `.go`, `.ts`, `.java`, `.rs`);
/// `tests/test_orders_api.py` is not.
fn is_own_test(test: &str, edited: &str) -> bool {
    let (Some(test), Some(edited)) = (base_name(test), base_name(edited)) else {
        return false;
    };
    let bare = TEST_PREFIXES
        .iter()
        .find_map(|prefix| test.strip_prefix(prefix))
        .or_else(|| {
            TEST_SUFFIXES
                .iter()
                .find_map(|suffix| test.strip_suffix(suffix))
        })
        .unwrap_or(test);
    !bare.is_empty() && bare.eq_ignore_ascii_case(edited)
}

/// The name of the file at `path` up to its first dot; none for a name that starts with one.
fn base_name(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next()?;
    name.split('.').next().filter(|stem| !stem.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each of these is named after the file it tests, and each of the others is not.
    #[test]
    fn a_test_file_is_known_by_its_name() {
        let own = [
            ("tests/test_orders.py", "src/orders.py"),
            ("pkg/orders_test.go", "pkg/orders.go"),
            ("src/orders.test.ts", "src/orders.ts"),
            ("src/orders.spec.ts", "src/orders.ts"),
            ("src/test/OrdersTest.java", "src/main/Orders.java"),
            ("test/OrdersTests.cs", "src/Orders.cs"),
            ("tests/orders.rs", "src/orders.rs"),
            ("spec/orders_spec.rb", "lib/orders.rb"),
        ];
        for (test, edited) in own {
            assert!(is_own_test(test, edited), "{test} tests {edited}");
        }
        let others = [
            ("tests/test_orders_api.py", "src/orders.py"),
            ("tests/test_billing.py", "src/orders.py"),
            ("tests/test_.py", "src/.py"),
            ("tests/.test.py", "src/orders.py"),
        ];
        for (test, edited) in others {
            assert!(!is_own_test(test, edited), "{test} does not test {edited}");
        }
    }
}
