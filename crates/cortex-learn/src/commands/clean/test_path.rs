//! The test file a command named, kept when the command itself cannot be. A word is taken for one
//! when, a test id after `::` taken off (`tests/test_ops.py::TestX::test_y` names
//! `tests/test_ops.py`), it is a relative path of letters, digits and `_-./` with no `..`, its
//! extension a source file's, and its name or a folder on the way says it holds tests: a name
//! starting `test` or ending `test`, `tests` or `spec` (`test_ops.py`, `ops_test.go`,
//! `OpsTest.java`, `ops.test.ts`, `ops_spec.rb`), or a folder named `test`, `tests`,
//! `__tests__`, `spec` or `specs`.
//!
//! The word may be one the shell expands, as long as what is kept is not: an unquoted test id with
//! brackets (`test_y[param-1]`) is a glob to the shell, but its file is plain. A word holding `$`
//! or a backquote is not taken, and neither is a path any detector finds a secret in.

use crate::commands::shell::Word;

use super::detectors;
use super::render::MAX_KEPT_CHARS;

/// What separates a test file from the test id after it.
pub(super) const TEST_ID_MARK: &str = "::";

/// The extensions of source files that hold tests.
const TEST_FILE_EXTENSIONS: &[&str] = &[
    "py", "rs", "go", "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "rb", "php", "java",
    "kt", "kts", "cs", "scala", "swift", "ex", "exs",
];

/// The folders whose files are tests.
const TEST_FOLDERS: &[&str] = &["test", "tests", "__tests__", "spec", "specs"];

/// The test file `word` names, if it names one.
pub(super) fn test_path(word: &Word) -> Option<String> {
    let text = word.text.as_str();
    if text.starts_with('-') || text.contains(['$', '`']) {
        return None;
    }
    let file = text.split(TEST_ID_MARK).next()?;
    if file.is_empty()
        || file.starts_with('/')
        || !file.bytes().all(is_path_byte)
        || file.len() > MAX_KEPT_CHARS
    {
        return None;
    }
    let names: Vec<&str> = file.split('/').collect();
    let (name, folders) = names.split_last()?;
    let (stem, extension) = name.rsplit_once('.')?;
    let is_test = is_test_name(stem) || folders.iter().any(|folder| TEST_FOLDERS.contains(folder));
    let is_kept = is_test
        && TEST_FILE_EXTENSIONS.contains(&extension)
        && !folders.contains(&"..")
        && detectors::find_all(file).is_empty();
    is_kept.then(|| file.to_owned())
}

/// Whether a file's name without its extension says it holds tests.
fn is_test_name(stem: &str) -> bool {
    let lower = stem.to_ascii_lowercase();
    lower.starts_with("test")
        || lower.ends_with("test")
        || lower.ends_with("tests")
        || lower.ends_with("spec")
}

/// A byte a kept path may hold: a letter, digit, `_`, `-`, `.` or `/`.
pub(super) fn is_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/')
}
