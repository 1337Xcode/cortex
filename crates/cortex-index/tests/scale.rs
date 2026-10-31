//! The scale mode a full index records: chosen from the repository's size, or pinned.

mod common;

use std::time::Duration;

use common::{TestResult, index, index_with_mode, write_all};
use cortex_core::{ModeSetting, ScaleMode};
use cortex_store::{MetaKey, Reader};

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// The scale mode stored for the repository at `root`.
fn stored_mode(root: &std::path::Path) -> TestResult<Option<String>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    Ok(reader.queries().meta(MetaKey::ScaleMode)?)
}

/// A repository of 50 Python files of 100 lines each, 5,000 lines in all, is small: neither under
/// 50 files nor under 5,000 lines, and at most 250 files. A vendored file and a generated one do
/// not count; with one file fewer it would be tiny. Pinned to medium, it is recorded as medium
/// whatever its size, and back on its own, small again.
#[test]
fn the_mode_follows_the_size_unless_pinned() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    let body = "x = 1\n".repeat(100);
    let mut files: Vec<(String, String)> = (0..50)
        .map(|number| (format!("src/m{number}.py"), body.clone()))
        .collect();
    files.push(("vendor/lib.py".to_owned(), body.clone()));
    files.push(("gen/out.py".to_owned(), format!("# @generated\n{body}")));
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    write_all(root, &borrowed)?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        (report.size.source_files, report.size.lines, report.mode),
        (50, 5_000, ScaleMode::Small)
    );
    assert_eq!(stored_mode(root)?.as_deref(), Some("small"));
    let pinned = ModeSetting::Pinned(ScaleMode::Medium);
    let report = index_with_mode(root, (2, GENEROUS, DEFAULT_LIMIT), pinned)?;
    assert_eq!(report.mode, ScaleMode::Medium);
    assert_eq!(stored_mode(root)?.as_deref(), Some("medium"));
    std::fs::remove_file(root.join("src/m0.py"))?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!(
        (report.size.source_files, report.mode),
        (49, ScaleMode::Tiny)
    );
    assert_eq!(stored_mode(root)?.as_deref(), Some("tiny"));
    Ok(())
}
