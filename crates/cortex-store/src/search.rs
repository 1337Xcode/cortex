//! Looking things up by name: how many search rows hold a word, the rows that best match a set of
//! words by BM25, the files whose path ends in a name, and the symbols of a name.
//!
//! A word reaches the full-text index only as a phrase in double quotes, its own quotes doubled,
//! so no word can change what the lookup asks; the lookup itself is a bound parameter like every
//! value the store passes.

use cortex_core::{Identifier, RepoPath};

use crate::error::{StoreError, sqlite};
use crate::ids::{FileId, NodeId};
use crate::queries::Queries;
use crate::schema;
use crate::values;

impl Queries<'_> {
    /// How many search rows hold `word`, as the index's vocabulary counts them; 0 for a word no
    /// row holds.
    ///
    /// # Errors
    ///
    /// SQLite errors, and a count that cannot be read back.
    pub fn term_rows(self, word: &str) -> Result<u64, StoreError> {
        let count: Option<i64> = self
            .connection()
            .prepare_cached(schema::SEARCH_TERM_ROWS)
            .and_then(|mut statement| {
                let mut rows = statement.query([word])?;
                rows.next()?.map(|row| row.get(0)).transpose()
            })
            .map_err(sqlite("counting the rows a word is in"))?;
        count.map_or(Ok(0), |count| {
            values::number(count, "search_terms.doc", count)
        })
    }

    /// The rows that best match any of `words`, best first, at most `limit`: each node with its
    /// BM25 score, the higher the better. Rows that score alike come in node order.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn search(self, words: &[&str], limit: usize) -> Result<Vec<(NodeId, f64)>, StoreError> {
        if words.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let expression = words
            .iter()
            .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.connection()
            .prepare_cached(schema::SEARCH_MATCH)
            .and_then(|mut statement| {
                statement
                    .query_map((expression, limit), |row| {
                        Ok((NodeId(row.get(0)?), row.get(1)?))
                    })?
                    .collect()
            })
            .map_err(sqlite("looking words up"))
    }

    /// Every file whose path is `name` or ends with `/` and `name`, by path, with its id and node.
    ///
    /// # Errors
    ///
    /// SQLite errors and stored paths that cannot be read back.
    pub fn files_named(self, name: &str) -> Result<Vec<(RepoPath, FileId, NodeId)>, StoreError> {
        let ending = format!("%/{}", like_escaped(name));
        let raw: Vec<(String, i64, i64)> = self
            .connection()
            .prepare_cached(schema::FILES_NAMED)
            .and_then(|mut statement| {
                statement
                    .query_map((name, ending), |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                    .collect()
            })
            .map_err(sqlite("finding files by name"))?;
        raw.into_iter()
            .map(|(path, file, node)| {
                let path = RepoPath::new(&path).map_err(|_| StoreError::Corrupt {
                    column: "files.path",
                    row: file,
                })?;
                Ok((path, FileId(file), NodeId(node)))
            })
            .collect()
    }

    /// The nodes of the first `limit` symbols named `name`, by node.
    ///
    /// # Errors
    ///
    /// SQLite errors.
    pub fn symbol_nodes_named(
        self,
        name: &Identifier,
        limit: usize,
    ) -> Result<Vec<NodeId>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.connection()
            .prepare_cached(schema::SYMBOL_NODES_NAMED)
            .and_then(|mut statement| {
                statement
                    .query_map((name.as_str(), limit), |row| Ok(NodeId(row.get(0)?)))?
                    .collect()
            })
            .map_err(sqlite("finding symbols by name"))
    }
}

/// `text` with the characters `LIKE` reads as wildcards, and its escape character, escaped by `\`.
fn like_escaped(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}
