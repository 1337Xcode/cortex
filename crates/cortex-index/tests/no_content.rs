//! What the index stores of real source: after a sample of every language Cortex reads is
//! indexed, no line of that source of 12 characters or more appears anywhere in the database file
//! or its log, the search index included, unless an empty store already holds it (the schema's
//! own text, such as `name TEXT NOT NULL`). Only the text of names is stored, and a Markdown
//! heading's text is its name.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{NOW_MS, TestResult, index, write};
use cortex_core::RepoId;
use cortex_store::{Writer, WriterOptions};

/// Lines shorter than this are too common to count as code, such as `return x;`.
const MIN_LINE_CHARS: usize = 12;

/// Lines of the samples that are names by design: the text of a Markdown heading underlined with
/// dashes is the heading's name.
const NAMES_BY_DESIGN: &[&str] = &["Setext title"];

/// Whether `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// The bytes of the database in the `.cortex` folder of `root`, and of its log if it has one.
fn database_bytes(root: &Path) -> TestResult<Vec<u8>> {
    let folder = root.join(".cortex");
    let mut stored = std::fs::read(folder.join("cortex.db"))?;
    let log = folder.join("cortex.db-wal");
    if log.try_exists()? {
        stored.extend(std::fs::read(log)?);
    }
    Ok(stored)
}

/// The bytes of a new store with nothing indexed: the schema and SQLite's own records.
fn empty_store_bytes() -> TestResult<Vec<u8>> {
    let empty = tempfile::tempdir()?;
    drop(Writer::open(&WriterOptions {
        folder: &empty.path().join(".cortex"),
        repo_id: &RepoId::of(empty.path()),
        mmap_bytes: 0,
        now_ms: NOW_MS,
    })?);
    database_bytes(empty.path())
}

/// Copies every language's sample under `root`, each into a folder named after the language, and
/// returns, per sample, its trimmed lines of [`MIN_LINE_CHARS`] or more that are not names by
/// design.
fn copy_samples(root: &Path) -> TestResult<Vec<Vec<String>>> {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/languages");
    let mut folders: Vec<_> = std::fs::read_dir(samples)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    folders.sort();
    let mut lines = Vec::new();
    for folder in folders {
        let language = folder
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("bad name")?;
        for entry in std::fs::read_dir(&folder)? {
            let file = entry?.path();
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("bad name")?;
            if !name.starts_with("sample.") {
                continue;
            }
            let text = std::fs::read_to_string(&file)?;
            write(root, &format!("{language}/{name}"), text.as_bytes())?;
            lines.push(
                text.lines()
                    .map(str::trim)
                    .filter(|line| line.chars().count() >= MIN_LINE_CHARS)
                    .filter(|line| !NAMES_BY_DESIGN.contains(line))
                    .map(str::to_owned)
                    .collect(),
            );
        }
    }
    Ok(lines)
}

/// Once every sample is indexed and the writer has closed, no source line appears in the
/// database file or its log, though the names were stored.
#[test]
fn no_source_line_reaches_the_database() -> TestResult {
    let repo = tempfile::tempdir()?;
    let samples = copy_samples(repo.path())?;
    // One sample per language Cortex reads, each with lines long enough to check.
    assert_eq!(samples.len(), 21);
    assert!(samples.iter().all(|lines| !lines.is_empty()));
    let lines: Vec<String> = samples.into_iter().flatten().collect();
    let report = index(repo.path(), 4, Duration::from_secs(60), 1_048_576)?;
    assert_eq!(report.parsed, 21, "{report:?}");
    let stored = database_bytes(repo.path())?;
    let empty = empty_store_bytes()?;
    // The names were stored, so the scan reads the data it is meant to check.
    assert!(contains(&stored, b"Order.__init__"));
    let leaked: Vec<&String> = lines
        .iter()
        .filter(|line| contains(&stored, line.as_bytes()))
        .filter(|line| !contains(&empty, line.as_bytes()))
        .collect();
    assert!(leaked.is_empty(), "stored: {leaked:#?}");
    Ok(())
}
