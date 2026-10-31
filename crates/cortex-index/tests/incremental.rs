//! Indexing again after changes: only changed files are read, written and parsed; the files
//! around a change are read again for what their imports bind; a Cortex of another version reads
//! everything again.

mod common;

use std::fs::File;
use std::path::Path;
use std::time::{Duration, SystemTime};

use common::{TestResult, expected, index, index_changed, ties, write_all};
use cortex_store::{ChangeKind, MetaKey, Reader};

/// A time limit no test file comes near, even in a debug build.
const GENEROUS: Duration = Duration::from_secs(60);

/// The default size limit, 1 MiB.
const DEFAULT_LIMIT: u64 = 1_048_576;

/// Sets the modification time of the file at `path` under `root` an hour from now.
fn touch(root: &Path, path: &str) -> TestResult {
    let later = SystemTime::now() + Duration::from_secs(3600);
    File::options()
        .write(true)
        .open(root.join(path))?
        .set_modified(later)?;
    Ok(())
}

/// The stored modification time of the file at `path` under `root`.
fn stored_mtime(root: &Path, path: &str) -> TestResult<Option<i64>> {
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    let row = reader.queries().file(&cortex_core::RepoPath::new(path)?)?;
    Ok(row.map(|row| row.file.mtime_ns))
}

/// Indexed again with nothing changed, every file is kept. A file with a new time and the same
/// contents gets only its new time; a changed file is written and parsed.
#[test]
fn only_changed_files_are_written() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("a.py", "def a():\n    pass\n"),
            ("b.py", "def b():\n    pass\n"),
            ("notes.txt", "notes\n"),
        ],
    )?;
    let first = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!((first.files, first.unchanged), (3, 0));
    let again = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!((again.files, again.parsed, again.unchanged), (0, 0, 3));
    let before = stored_mtime(root, "a.py")?;
    touch(root, "a.py")?;
    write_all(root, &[("b.py", "def b():\n    return 1\n")])?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!((report.files, report.parsed, report.unchanged), (1, 1, 2));
    let after = stored_mtime(root, "a.py")?;
    assert!(after > before, "{before:?} then {after:?}");
    Ok(())
}

/// A file whose imports name a changed file is read again and its references tied again. `app.py`
/// imports `helper` from `lib.py`, which renames it; `app.py` is not changed, and its call, bound
/// by the import, is left unresolved, not tied by its name to `other.py`'s `helper`.
#[test]
fn importers_of_a_changed_file_are_tied_again() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("lib.py", "def helper():\n    pass\n"),
            ("other.py", "def helper():\n    pass\n"),
            ("app.py", "from lib import helper\n\nhelper()\n"),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let helper = Some(("lib.py", "helper", "import_path"));
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "helper", 1, helper),
            ("call", "helper", 3, helper)
        ])
    );
    write_all(root, &[("lib.py", "def renamed():\n    pass\n")])?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!((report.files, report.unchanged), (1, 2));
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[("import", "helper", 1, None), ("call", "helper", 3, None)])
    );
    Ok(())
}

/// A file around a change that has itself changed since it was stored, its own change not yet
/// indexed, is not read again: its stored references belong to its stored version, and the
/// imports of its new one would be matched to them in the wrong order. `app.py` takes `helper`
/// from `lib.py`; `lib.py` drops `helper` while `app.py` gains an import of `thing` and a call of
/// it before its call of `helper`. Indexing `lib.py` leaves `app.py`'s call of `helper` untied,
/// not tied to `thing`; indexing `app.py` then ties its new import and call.
#[test]
fn a_file_changed_but_not_yet_indexed_is_not_read_again() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("lib.py", "def helper():\n    pass\n"),
            ("other.py", "def thing():\n    pass\n"),
            ("app.py", "from lib import helper\n\nhelper()\n"),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    write_all(
        root,
        &[
            ("lib.py", "def renamed():\n    pass\n"),
            (
                "app.py",
                "from lib import helper\nfrom other import thing\n\nthing()\nhelper()\n",
            ),
        ],
    )?;
    index_changed(root, &["lib.py"])?;
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[("import", "helper", 1, None), ("call", "helper", 3, None)])
    );
    index_changed(root, &["app.py"])?;
    let thing = Some(("other.py", "thing", "import_path"));
    assert_eq!(
        ties(root, "app.py")?,
        expected(&[
            ("import", "helper", 1, None),
            ("import", "thing", 2, thing),
            ("call", "thing", 4, thing),
            ("call", "helper", 5, None),
        ])
    );
    Ok(())
}

/// A file a changed file reaches through its imports is read again for what it passes on.
/// `app.ts` takes `x` from `index.ts`, which passes it on from `lib.ts`; after `app.ts` changes,
/// `index.ts` is not written again, but `x` is still tied to `lib.ts`'s through it.
#[test]
fn names_still_pass_through_unchanged_files() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[
            ("src/lib.ts", "export function x() {}\n"),
            ("src/index.ts", "export { x } from \"./lib\";\n"),
            ("src/app.ts", "import { x } from \"./index\";\nx();\n"),
        ],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    write_all(
        root,
        &[("src/app.ts", "import { x } from \"./index\";\n\nx();\n")],
    )?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!((report.files, report.unchanged), (1, 2));
    let x = Some(("src/lib.ts", "x", "import_path"));
    assert_eq!(
        ties(root, "src/app.ts")?,
        expected(&[("import", "x", 1, x), ("call", "x", 3, x)])
    );
    Ok(())
}

/// An index written by another version of Cortex is read again whole: every file is written.
#[test]
fn another_version_reads_every_file_again() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(root, &[("a.py", "x = 1\n"), ("b.txt", "b\n")])?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    rusqlite::Connection::open(root.join(".cortex").join("cortex.db"))?
        .execute_batch("UPDATE meta SET value = '0.0.1' WHERE key = 'cortex_version'")?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    assert_eq!((report.files, report.unchanged), (2, 0));
    let reader = Reader::open(&root.join(".cortex"), 0)?;
    assert_ne!(
        reader.queries().meta(MetaKey::CortexVersion)?.as_deref(),
        Some("0.0.1")
    );
    Ok(())
}

/// A changed file reports the definitions whose signature or body changed: `total` gains a
/// parameter (its signature), `place` returns something else (its body), `extra` is new and
/// `keep` is as it was, so neither is reported.
#[test]
fn changed_definitions_are_reported() -> TestResult {
    let repo = tempfile::tempdir()?;
    let root = repo.path();
    write_all(
        root,
        &[(
            "shop.py",
            "def total(items):\n    return 0\n\n\ndef place():\n    return 1\n\n\n\
             def keep():\n    return 2\n",
        )],
    )?;
    index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    write_all(
        root,
        &[(
            "shop.py",
            "def total(items, tax):\n    return 0\n\n\ndef place():\n    return 3\n\n\n\
             def keep():\n    return 2\n\n\ndef extra():\n    pass\n",
        )],
    )?;
    let report = index(root, 2, GENEROUS, DEFAULT_LIMIT)?;
    let changed: Vec<(&str, ChangeKind)> = report
        .changed_symbols
        .iter()
        .map(|change| (change.name.as_str(), change.kind))
        .collect();
    assert_eq!(
        changed,
        [
            ("total", ChangeKind::Signature),
            ("place", ChangeKind::Body)
        ]
    );
    Ok(())
}
