//! Which source file a test file tests when its imports do not say. A test that imports source
//! files is tied to them by its import edges, most strongly to the one it imports most. One that
//! imports none, as a Go test beside its package's files or a test that reaches its target by a
//! path no resolver follows, is the test of the file whose name its own spells without the test
//! marks: `orders_test.go` tests `orders.go`, `test_orders.py` tests `orders.py` and
//! `orders.test.ts` tests `orders.ts`. The file of that name in the test's own folder is the one;
//! else the only one in the repository; else there is none. A file that is a test only by its
//! folder has no test marks to take off, and its name, often one many files share (`mod.rs`,
//! `helpers.py`), says nothing of what it tests: it gets no target.

use cortex_core::RepoPath;

use crate::file_edges::ImportCounts;
use crate::imports::Project;
use crate::path_rules::{NamePattern, is_test_file};

/// The file the file at `test` tests by its base name: when its name matches one of its language's
/// test names `names` and `counts`, the files its imports name, holds no source file. `None` when
/// it is no such test, or no single file has the name.
pub(crate) fn base_name_target(
    test: &RepoPath,
    names: &[NamePattern],
    counts: &ImportCounts,
    project: &Project,
) -> Option<RepoPath> {
    let is_source = |path: &RepoPath| path != test && !is_test_file(path, names);
    if !is_test_file(test, names) || counts.keys().any(is_source) {
        return None;
    }
    let name = target_name(test.file_name(), names)?;
    let candidates: Vec<&RepoPath> = project
        .files_named(&name)
        .iter()
        .filter(|path| is_source(path))
        .collect();
    let folder = test.parent();
    let beside: Vec<&RepoPath> = candidates
        .iter()
        .copied()
        .filter(|path| path.parent() == folder)
        .collect();
    match (beside.as_slice(), candidates.as_slice()) {
        ([only], _) | ([], [only]) => Some((*only).clone()),
        _ => None,
    }
}

/// The name of the file a test file named `name` tests: without the part the first of `names`
/// that matches it spells, keeping the extension a suffix holds. `None` when no pattern matches,
/// or nothing would be left.
fn target_name(name: &str, names: &[NamePattern]) -> Option<String> {
    for pattern in names {
        match *pattern {
            NamePattern::Prefix(prefix) => {
                if let Some(rest) = name.strip_prefix(prefix) {
                    return (!rest.is_empty()).then(|| rest.to_owned());
                }
            }
            NamePattern::Suffix(suffix) => {
                if let Some(stem) = name.strip_suffix(suffix) {
                    let (_, extension) = suffix.rsplit_once('.')?;
                    return (!stem.is_empty()).then(|| format!("{stem}.{extension}"));
                }
            }
            NamePattern::Exact(_) | NamePattern::Contains(_) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test mark is taken off the name, a suffix's extension kept: a prefix (`test_`), a
    /// suffix with a mark of its own (`_test.go`, `.test.ts`, `Tests.cs` before `Test.cs`). A
    /// name that is all mark leaves nothing, and a name no pattern matches, a test by its folder
    /// alone, names no file.
    #[test]
    fn test_marks_are_taken_off_the_name() {
        let python = [
            NamePattern::Prefix("test_"),
            NamePattern::Suffix("_test.py"),
        ];
        let csharp = [
            NamePattern::Suffix("Tests.cs"),
            NamePattern::Suffix("Test.cs"),
        ];
        let cases: [(&str, &[NamePattern], Option<&str>); 8] = [
            ("test_orders.py", &python, Some("orders.py")),
            ("orders_test.py", &python, Some("orders.py")),
            ("test_", &python, None),
            ("orders.py", &python, None),
            (
                "orders_test.go",
                &[NamePattern::Suffix("_test.go")],
                Some("orders.go"),
            ),
            (
                "cart.test.ts",
                &[NamePattern::Suffix(".test.ts")],
                Some("cart.ts"),
            ),
            ("OrdersTests.cs", &csharp, Some("Orders.cs")),
            ("OrdersTest.cs", &csharp, Some("Orders.cs")),
        ];
        for (name, names, expected) in cases {
            assert_eq!(target_name(name, names).as_deref(), expected, "{name}");
        }
    }
}
