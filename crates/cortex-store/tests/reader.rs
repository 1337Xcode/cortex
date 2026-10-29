//! Readers: what they need before they open, and what they see beside the writer.

mod common;

use common::{Scratch, TestResult, index, path, symbol};
use cortex_core::SymbolKind;
use cortex_store::StoreError;
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
                expected: 1
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
