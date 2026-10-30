//! The schema: every migration, and every statement that reads or writes the tables.
//!
//! Table and column names appear only here and in `migrations/`. Values always travel as bound
//! parameters; no statement is ever built by formatting text.

/// One schema change.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Migration {
    /// Its number: the schema version once it has run. Numbers start at 1 and rise by 1.
    pub(crate) version: u32,
    /// The statements that make the change.
    pub(crate) sql: &'static str,
}

/// Every migration, in order. A migration that has been published is never edited; a change is a
/// new migration at the end.
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("../migrations/0001_index.sql"),
}];

/// The schema version this build reads and writes: the last migration's number.
pub(crate) const SCHEMA_VERSION: u32 = 1;

/// How many objects the database defines; 0 for a new, empty database.
pub(crate) const COUNT_OBJECTS: &str = "SELECT count(*) FROM sqlite_schema";
/// Whether the `meta` table exists (1) or not (0).
pub(crate) const COUNT_META_TABLES: &str =
    "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'meta'";
/// The value of a `meta` key.
pub(crate) const META_GET: &str = "SELECT value FROM meta WHERE key = ?1";
/// Sets a `meta` key.
pub(crate) const META_SET: &str = "INSERT INTO meta (key, value) VALUES (?1, ?2) \
     ON CONFLICT (key) DO UPDATE SET value = excluded.value";
/// Sets a `meta` key unless it already has a value.
pub(crate) const META_SET_ONCE: &str =
    "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO NOTHING";

/// The id of the file at a path.
pub(crate) const FILE_ID: &str = "SELECT file_id FROM files WHERE path = ?1";
/// Adds a file and returns its id.
pub(crate) const FILE_INSERT: &str = "INSERT INTO files \
     (path, lang, bytes, loc, content_hash, mtime_ns, flags, indexed_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) RETURNING file_id";
/// Rewrites a file's row, found by path; the parameters are those of [`FILE_INSERT`].
pub(crate) const FILE_UPDATE: &str = "UPDATE files SET lang = ?2, bytes = ?3, loc = ?4, \
     content_hash = ?5, mtime_ns = ?6, flags = ?7, indexed_at = ?8 WHERE path = ?1";
/// Deletes a file's row.
pub(crate) const FILE_DELETE: &str = "DELETE FROM files WHERE file_id = ?1";
/// A file's row, found by path.
pub(crate) const FILE_BY_PATH: &str = "SELECT file_id, path, lang, bytes, loc, content_hash, \
     mtime_ns, flags, indexed_at FROM files WHERE path = ?1";
/// Every file's id and path, by path. Text compares byte by byte here, as Rust strings do.
pub(crate) const FILE_PATHS: &str = "SELECT file_id, path FROM files ORDER BY path";

/// What a file's stored symbols are matched by, oldest first.
pub(crate) const SYMBOLS_TO_MATCH: &str = "SELECT symbol_id, kind, name, qual_name, sig_hash, \
     body_hash FROM symbols WHERE file_id = ?1 ORDER BY symbol_id";
/// Adds a symbol and returns its id.
pub(crate) const SYMBOL_INSERT: &str = "INSERT INTO symbols (file_id, parent_id, kind, name, \
     qual_name, start_byte, end_byte, start_line, end_line, sig_hash, body_hash) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) RETURNING symbol_id";
/// Rewrites what may change in a matched symbol, found by id.
pub(crate) const SYMBOL_UPDATE: &str = "UPDATE symbols SET parent_id = ?2, name = ?3, \
     start_byte = ?4, end_byte = ?5, start_line = ?6, end_line = ?7, sig_hash = ?8, \
     body_hash = ?9 WHERE symbol_id = ?1";
/// Deletes a symbol's row.
pub(crate) const SYMBOL_DELETE: &str = "DELETE FROM symbols WHERE symbol_id = ?1";
/// The columns of a symbol row, as [`crate::queries`] reads them.
macro_rules! symbol_columns {
    () => {
        "SELECT symbol_id, file_id, parent_id, kind, name, qual_name, start_byte, end_byte, \
         start_line, end_line, sig_hash, body_hash FROM symbols "
    };
}
/// A file's symbols in the order they appear.
pub(crate) const SYMBOLS_IN_FILE: &str = concat!(
    symbol_columns!(),
    "WHERE file_id = ?1 ORDER BY start_byte, symbol_id"
);
/// The symbols with a name, oldest first.
pub(crate) const SYMBOLS_NAMED: &str =
    concat!(symbol_columns!(), "WHERE name = ?1 ORDER BY symbol_id");

/// Adds an unresolved reference and returns its id.
pub(crate) const REF_INSERT: &str = "INSERT INTO refs (src_file_id, src_symbol_id, dst_name, \
     dst_symbol_id, kind, line, resolution, confidence) \
     VALUES (?1, ?2, ?3, NULL, ?4, ?5, NULL, 0.0) RETURNING ref_id";
/// Deletes every reference in a file.
pub(crate) const REFS_DELETE_IN_FILE: &str = "DELETE FROM refs WHERE src_file_id = ?1";
/// Marks every reference to a symbol unresolved.
pub(crate) const REFS_UNRESOLVE_TO: &str = "UPDATE refs SET dst_symbol_id = NULL, \
     resolution = NULL, confidence = 0.0 WHERE dst_symbol_id = ?1";
/// Sets a reference's target, resolution and confidence, found by id.
pub(crate) const REF_RESOLVE: &str = "UPDATE refs SET dst_symbol_id = ?2, resolution = ?3, \
     confidence = ?4 WHERE ref_id = ?1";
/// A file's references in the order they were written.
pub(crate) const REFS_IN_FILE: &str = "SELECT ref_id, src_file_id, src_symbol_id, dst_name, \
     dst_symbol_id, kind, line, resolution, confidence FROM refs WHERE src_file_id = ?1 \
     ORDER BY ref_id";

/// Adds the node for a row, or updates its path, and returns its id.
pub(crate) const NODE_UPSERT: &str = "INSERT INTO nodes (kind, ref_id, path) VALUES (?1, ?2, ?3) \
     ON CONFLICT (kind, ref_id) DO UPDATE SET path = excluded.path RETURNING node_id";
/// The node of a row.
pub(crate) const NODE_OF: &str = "SELECT node_id FROM nodes WHERE kind = ?1 AND ref_id = ?2";
/// Deletes a node.
pub(crate) const NODE_DELETE: &str = "DELETE FROM nodes WHERE node_id = ?1";

/// Deletes a node's outgoing edges of one type.
pub(crate) const EDGES_DELETE_FROM: &str = "DELETE FROM edges WHERE src = ?1 AND type = ?2";
/// Adds an edge.
pub(crate) const EDGE_INSERT: &str = "INSERT INTO edges (src, dst, type, w, n, last_seq, directed) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";
/// A node's outgoing edges, by type, then target.
pub(crate) const EDGES_FROM: &str = "SELECT src, dst, type, w, n, last_seq, directed FROM edges \
     WHERE src = ?1 ORDER BY type, dst";

/// Row counts for status.
pub(crate) const COUNTS: &str = "SELECT (SELECT count(*) FROM files), \
     (SELECT count(*) FROM symbols), (SELECT count(*) FROM refs), \
     (SELECT count(*) FROM refs WHERE dst_symbol_id IS NOT NULL), \
     (SELECT count(*) FROM nodes), (SELECT count(*) FROM edges)";

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    /// One column as `PRAGMA table_info` reports it: name, declared type, position in the primary
    /// key (0 when not part of it).
    type Column = (String, String, i64);

    /// One index: its columns in order, and whether it is unique.
    type Index = (Vec<String>, bool);

    /// The columns of `table`.
    fn columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<Column>> {
        let mut statement =
            connection.prepare("SELECT name, type, pk FROM pragma_table_info(?1)")?;
        statement
            .query_map([table], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect()
    }

    /// The indexes of `table`, sorted.
    fn indexes(connection: &Connection, table: &str) -> rusqlite::Result<Vec<Index>> {
        let mut statement =
            connection.prepare(r#"SELECT name, "unique" FROM pragma_index_list(?1)"#)?;
        let listed: Vec<(String, bool)> = statement
            .query_map([table], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut found = Vec::new();
        for (name, is_unique) in listed {
            let mut statement =
                connection.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
            let names: Vec<String> = statement
                .query_map([name], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            found.push((names, is_unique));
        }
        found.sort();
        Ok(found)
    }

    /// A column list typed by hand.
    fn column_list(entries: &[(&str, &str, i64)]) -> Vec<Column> {
        entries
            .iter()
            .map(|(name, kind, pk)| ((*name).to_owned(), (*kind).to_owned(), *pk))
            .collect()
    }

    /// An index list typed by hand.
    fn index_list(entries: &[(&[&str], bool)]) -> Vec<Index> {
        let mut list: Vec<Index> = entries
            .iter()
            .map(|(names, is_unique)| {
                let names = names.iter().map(|name| (*name).to_owned()).collect();
                (names, *is_unique)
            })
            .collect();
        list.sort();
        list
    }

    /// The columns of every table, typed from the index's table definitions.
    fn expected_columns() -> [(&'static str, Vec<Column>); 7] {
        [
            (
                "meta",
                column_list(&[("key", "TEXT", 1), ("value", "TEXT", 0)]),
            ),
            (
                "files",
                column_list(&[
                    ("file_id", "INTEGER", 1),
                    ("path", "TEXT", 0),
                    ("lang", "TEXT", 0),
                    ("bytes", "INTEGER", 0),
                    ("loc", "INTEGER", 0),
                    ("content_hash", "BLOB", 0),
                    ("mtime_ns", "INTEGER", 0),
                    ("flags", "INTEGER", 0),
                    ("indexed_at", "INTEGER", 0),
                ]),
            ),
            (
                "symbols",
                column_list(&[
                    ("symbol_id", "INTEGER", 1),
                    ("file_id", "INTEGER", 0),
                    ("parent_id", "INTEGER", 0),
                    ("kind", "TEXT", 0),
                    ("name", "TEXT", 0),
                    ("qual_name", "TEXT", 0),
                    ("start_byte", "INTEGER", 0),
                    ("end_byte", "INTEGER", 0),
                    ("start_line", "INTEGER", 0),
                    ("end_line", "INTEGER", 0),
                    ("sig_hash", "BLOB", 0),
                    ("body_hash", "BLOB", 0),
                ]),
            ),
            (
                "refs",
                column_list(&[
                    ("ref_id", "INTEGER", 1),
                    ("src_file_id", "INTEGER", 0),
                    ("src_symbol_id", "INTEGER", 0),
                    ("dst_name", "TEXT", 0),
                    ("dst_symbol_id", "INTEGER", 0),
                    ("kind", "TEXT", 0),
                    ("line", "INTEGER", 0),
                    ("resolution", "TEXT", 0),
                    ("confidence", "REAL", 0),
                ]),
            ),
            (
                "nodes",
                column_list(&[
                    ("node_id", "INTEGER", 1),
                    ("kind", "TEXT", 0),
                    ("ref_id", "INTEGER", 0),
                    ("path", "TEXT", 0),
                ]),
            ),
            (
                "edges",
                column_list(&[
                    ("src", "INTEGER", 1),
                    ("dst", "INTEGER", 2),
                    ("type", "INTEGER", 3),
                    ("w", "REAL", 0),
                    ("n", "INTEGER", 0),
                    ("last_seq", "INTEGER", 0),
                    ("directed", "INTEGER", 0),
                ]),
            ),
            (
                "search_index",
                column_list(&[("node_id", "", 0), ("tokens", "", 0)]),
            ),
        ]
    }

    /// The indexes of every table: those of the table definitions, plus `refs.dst_name`, `nodes.path`
    /// and the uniqueness of folder paths.
    fn expected_indexes() -> [(&'static str, Vec<Index>); 6] {
        [
            ("meta", index_list(&[(&["key"], true)])),
            (
                "files",
                index_list(&[(&["path"], true), (&["lang"], false)]),
            ),
            (
                "symbols",
                index_list(&[
                    (&["file_id"], false),
                    (&["name"], false),
                    (&["qual_name"], false),
                ]),
            ),
            (
                "refs",
                index_list(&[
                    (&["dst_symbol_id"], false),
                    (&["src_file_id"], false),
                    (&["dst_name"], false),
                ]),
            ),
            (
                "nodes",
                index_list(&[
                    (&["kind", "ref_id"], true),
                    (&["path"], false),
                    (&["path"], true),
                ]),
            ),
            (
                "edges",
                index_list(&[(&["src", "dst", "type"], true), (&["dst", "type"], false)]),
            ),
        ]
    }

    /// Migration numbers start at 1, rise by 1, and end at the version this build reads.
    #[test]
    fn migrations_are_numbered_in_order() {
        let numbers: Vec<u32> = MIGRATIONS
            .iter()
            .map(|migration| migration.version)
            .collect();
        let expected: Vec<u32> = (1..=SCHEMA_VERSION).collect();
        assert_eq!(numbers, expected);
    }

    /// The foreign keys of `table`: referenced table, column, referenced column and what a delete
    /// does, sorted.
    fn foreign_keys(
        connection: &Connection,
        table: &str,
    ) -> rusqlite::Result<Vec<(String, String, String, String)>> {
        let mut statement = connection.prepare(
            r#"SELECT "table", "from", "to", on_delete FROM pragma_foreign_key_list(?1)"#,
        )?;
        let mut keys: Vec<(String, String, String, String)> = statement
            .query_map([table], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        keys.sort();
        Ok(keys)
    }

    /// A foreign key typed by hand.
    fn key(table: &str, from: &str, to: &str, on_delete: &str) -> (String, String, String, String) {
        (
            table.to_owned(),
            from.to_owned(),
            to.to_owned(),
            on_delete.to_owned(),
        )
    }

    /// The tables, columns, keys and indexes are the table definitions typed below: names, types
    /// and primary keys as the index's tables define them. Two indexes are added, `refs.dst_name`
    /// for resolving references by name and `nodes.path` for finding the nodes under a path, and
    /// folder paths are unique. Foreign keys refuse references to missing rows, and an edge goes
    /// with either of its nodes.
    #[test]
    fn schema_matches_the_table_definitions() -> Result<(), Box<dyn std::error::Error>> {
        let connection = Connection::open_in_memory()?;
        for migration in MIGRATIONS {
            connection.execute_batch(migration.sql)?;
        }
        let mut statement = connection.prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             AND name NOT LIKE 'search_index_%' ORDER BY name",
        )?;
        let tables: Vec<String> = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        assert_eq!(
            tables,
            [
                "edges",
                "files",
                "meta",
                "nodes",
                "refs",
                "search_index",
                "symbols"
            ]
        );
        for (table, expected) in &expected_columns() {
            assert_eq!(&columns(&connection, table)?, expected, "{table}");
        }
        for (table, expected) in &expected_indexes() {
            assert_eq!(&indexes(&connection, table)?, expected, "{table}");
        }
        assert_eq!(
            foreign_keys(&connection, "symbols")?,
            [key("files", "file_id", "file_id", "NO ACTION")]
        );
        assert_eq!(
            foreign_keys(&connection, "refs")?,
            [
                key("files", "src_file_id", "file_id", "NO ACTION"),
                key("symbols", "dst_symbol_id", "symbol_id", "NO ACTION"),
            ]
        );
        assert_eq!(
            foreign_keys(&connection, "edges")?,
            [
                key("nodes", "dst", "node_id", "CASCADE"),
                key("nodes", "src", "node_id", "CASCADE"),
            ]
        );
        Ok(())
    }
}
