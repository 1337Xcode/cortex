//! Readers: what they need before they open, and what they see beside the writer.

mod common;

use common::{Scratch, TestResult, index, path, reference, symbol};
use cortex_core::{RefKind, Resolution, SymbolKind};
use cortex_store::{Resolved, StoreError, Target};
use rusqlite::ErrorCode;

/// Without a database there is nothing to read, and the error says where it was looked for.
#[test]
fn a_reader_needs_a_database() -> TestResult {
    let scratch = Scratch::new()?;
    let error = scratch.reader().err();
    let expected = scratch.folder.join("cortex.db");
    assert!(
        matches!(&error, Some(StoreError::Missing { path }) if *path == expected),
        "{error:?}"
    );
    Ok(())
}

/// A reader opened beside the writer sees a transaction's changes once it commits, not before.
#[test]
fn a_reader_sees_only_committed_changes() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let reader = scratch.reader()?;
    let file = index(
        "src/a.py",
        vec![symbol(SymbolKind::Function, "run", None, ("s", "b"))?],
        Vec::new(),
    )?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&file)?;
    assert_eq!(reader.queries().file(&path("src/a.py")?)?, None);
    transaction.commit()?;
    let row = reader.queries().file(&path("src/a.py")?)?;
    assert_eq!(row.map(|row| row.file), Some(file.file));
    Ok(())
}

/// A reader opens a database whose writer has closed, and reads what it committed.
#[test]
fn a_reader_opens_after_the_writer_closed() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    transaction.replace_file(&index("src/a.py", Vec::new(), Vec::new())?)?;
    transaction.commit()?;
    drop(writer);
    let reader = scratch.reader()?;
    assert_eq!(reader.queries().counts()?.files, 1);
    Ok(())
}

/// An empty database file, which the writer has not set up yet, is reported as schema version 0.
#[test]
fn a_reader_refuses_a_database_without_the_schema() -> TestResult {
    let scratch = Scratch::new()?;
    std::fs::create_dir_all(&scratch.folder)?;
    std::fs::write(scratch.folder.join("cortex.db"), b"")?;
    let error = scratch.reader().err();
    assert!(
        matches!(
            error,
            Some(StoreError::SchemaNotCurrent {
                found: 0,
                expected: 2
            })
        ),
        "{error:?}"
    );
    Ok(())
}

/// A file that is not a database is refused by the writer and by readers, and left as it was.
#[test]
fn a_damaged_database_is_refused() -> TestResult {
    let scratch = Scratch::new()?;
    std::fs::create_dir_all(&scratch.folder)?;
    let database = scratch.folder.join("cortex.db");
    let junk = b"this is not a database, only text that fills its first page ".repeat(80);
    std::fs::write(&database, &junk)?;
    for error in [scratch.writer(1).err(), scratch.reader().err()] {
        assert!(
            matches!(
                &error,
                Some(StoreError::Sqlite {
                    source: rusqlite::Error::SqliteFailure(failure, _),
                    ..
                }) if failure.code == ErrorCode::NotADatabase
            ),
            "{error:?}"
        );
    }
    assert_eq!(std::fs::read(&database)?, junk);
    Ok(())
}

/// The column and row a damaged value is reported by, if it is reported as damaged.
fn corrupt<T>(read: Result<T, StoreError>) -> Option<(&'static str, i64)> {
    match read {
        Err(StoreError::Corrupt { column, row }) => Some((column, row)),
        _ => None,
    }
}

/// A damaged value in a name row is reported with its column and row, never read as something
/// else: a language Cortex does not know (by the file's id), a name that is not one line and a
/// kind Cortex does not know (by the symbol's or the reference's id), a target without its
/// resolution, and a confidence that is not the resolution's. Each value is damaged on its own and repaired before the next.
#[test]
fn damaged_name_rows_are_reported() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let mut transaction = writer.transaction()?;
    let change = transaction.replace_file(&index(
        "src/a.py",
        vec![symbol(SymbolKind::Function, "helper", None, ("s", "b"))?],
        vec![reference(Some(0), "helper", RefKind::Call, 2)?],
    )?)?;
    let target = Target {
        symbol: change.symbols[0],
        resolution: Resolution::SameFile,
    };
    transaction.set_resolutions(&[Resolved {
        reference: change.refs[0],
        target: Some(target),
        goes_through_import: false,
    }])?;
    transaction.commit()?;
    drop(writer);
    let (file, symbol_id, ref_id) = (
        change.file.get(),
        change.symbols[0].get(),
        change.refs[0].get(),
    );
    let damage = |sql: &str| -> TestResult {
        rusqlite::Connection::open(scratch.folder.join("cortex.db"))?.execute_batch(sql)?;
        Ok(())
    };

    damage("UPDATE files SET lang = 'cobol'")?;
    let reader = scratch.reader()?;
    assert_eq!(
        corrupt(reader.queries().symbol_names()),
        Some(("files.lang", file))
    );
    assert_eq!(
        corrupt(reader.queries().ref_names()),
        Some(("files.lang", file))
    );
    damage("UPDATE files SET lang = 'python'")?;

    damage("UPDATE symbols SET name = 'two' || char(10) || 'lines'")?;
    let damaged = corrupt(scratch.reader()?.queries().symbol_names());
    assert_eq!(damaged, Some(("symbols.name", symbol_id)));
    damage("UPDATE symbols SET name = 'helper'")?;

    damage("UPDATE symbols SET kind = 'functions'")?;
    let damaged = corrupt(scratch.reader()?.queries().symbol_names());
    assert_eq!(damaged, Some(("symbols.kind", symbol_id)));
    damage("UPDATE symbols SET kind = 'function'")?;

    damage("UPDATE refs SET dst_name = 'two' || char(10) || 'lines'")?;
    let damaged = corrupt(scratch.reader()?.queries().ref_names());
    assert_eq!(damaged, Some(("refs.dst_name", ref_id)));
    damage("UPDATE refs SET dst_name = 'helper'")?;

    damage("UPDATE refs SET kind = 'calls'")?;
    let damaged = corrupt(scratch.reader()?.queries().ref_names());
    assert_eq!(damaged, Some(("refs.kind", ref_id)));
    damage("UPDATE refs SET kind = 'call'")?;

    damage("UPDATE refs SET line = -1")?;
    let damaged = corrupt(scratch.reader()?.queries().ref_names());
    assert_eq!(damaged, Some(("refs.line", ref_id)));
    damage("UPDATE refs SET line = 2")?;

    damage("UPDATE refs SET resolution = NULL")?;
    let damaged = corrupt(scratch.reader()?.queries().ref_names());
    assert_eq!(damaged, Some(("refs.resolution", ref_id)));
    damage("UPDATE refs SET resolution = 'same_file'")?;

    damage("UPDATE refs SET confidence = 0.5")?;
    let reader = scratch.reader()?;
    let damaged = corrupt(reader.queries().ref_names());
    assert_eq!(damaged, Some(("refs.confidence", ref_id)));
    let damaged = corrupt(reader.queries().refs_in_file(change.file));
    assert_eq!(damaged, Some(("refs.confidence", ref_id)));
    damage("UPDATE refs SET confidence = 0.6")?;

    let names = scratch.reader()?.queries().ref_names()?;
    assert_eq!(names.first().and_then(|name| name.target), Some(target));
    Ok(())
}
