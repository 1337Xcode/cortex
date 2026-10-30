//! How values are stored and read back: unsigned numbers as SQLite integers, kinds by their
//! stored names, names and paths as text checked again on the way out.

use cortex_core::{Identifier, Language, RepoPath};

use crate::error::StoreError;

/// `value` as a database integer for `column`.
pub(crate) fn integer(value: u64, column: &'static str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::TooLarge { column, value })
}

/// A stored integer read back as a `T`, such as a count or a line number.
pub(crate) fn number<T: TryFrom<i64>>(
    value: i64,
    column: &'static str,
    row: i64,
) -> Result<T, StoreError> {
    T::try_from(value).map_err(|_| StoreError::Corrupt { column, row })
}

/// A stored name read back.
pub(crate) fn identifier(
    text: &str,
    column: &'static str,
    row: i64,
) -> Result<Identifier, StoreError> {
    Identifier::new(text).map_err(|_| StoreError::Corrupt { column, row })
}

/// A stored path read back.
pub(crate) fn path(text: &str, column: &'static str, row: i64) -> Result<RepoPath, StoreError> {
    RepoPath::new(text).map_err(|_| StoreError::Corrupt { column, row })
}

/// A stored kind read back through its type's `from_name`.
pub(crate) fn kind<T>(
    text: &str,
    from_name: fn(&str) -> Option<T>,
    column: &'static str,
    row: i64,
) -> Result<T, StoreError> {
    from_name(text).ok_or(StoreError::Corrupt { column, row })
}

/// A file's stored language read back, absent when the file has none; `row` is the file's id.
pub(crate) fn language(name: Option<String>, row: i64) -> Result<Option<Language>, StoreError> {
    name.map(|name| kind(&name, Language::from_name, "files.lang", row))
        .transpose()
}
