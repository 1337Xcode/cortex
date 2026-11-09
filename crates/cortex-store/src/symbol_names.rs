//! Finding symbols by the name a person or an agent gives: the symbols whose qualified name it is,
//! or, when none has it as its qualified name, those whose plain name it is. So `Order.create`
//! names one method, and `create` every definition called that, wherever it is nested.

use crate::error::StoreError;
use crate::ids::FileId;
use crate::queries::Queries;
use crate::rows::SymbolRow;
use crate::schema;

impl Queries<'_> {
    /// The symbols of `file` that `name` names, in the order they appear.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn symbols_in_file_called(
        self,
        file: FileId,
        name: &str,
    ) -> Result<Vec<SymbolRow>, StoreError> {
        Ok(called(self.symbols_in_file(file)?, name))
    }

    /// The symbols anywhere in the repository that `name` names, oldest first.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored values that cannot be read back.
    pub fn symbols_called(self, name: &str) -> Result<Vec<SymbolRow>, StoreError> {
        Ok(called(self.symbols(schema::SYMBOLS_CALLED, name)?, name))
    }
}

/// Of `rows`, those whose qualified name is `name`, or when there are none, those whose name is.
fn called(rows: Vec<SymbolRow>, name: &str) -> Vec<SymbolRow> {
    let (qualified, plain): (Vec<SymbolRow>, Vec<SymbolRow>) = rows
        .into_iter()
        .filter(|row| row.qual_name.as_str() == name || row.name.as_str() == name)
        .partition(|row| row.qual_name.as_str() == name);
    if qualified.is_empty() {
        plain
    } else {
        qualified
    }
}
