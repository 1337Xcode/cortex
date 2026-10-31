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
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../migrations/0001_index.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../migrations/0002_passes_on.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("../migrations/0003_refs_through_imports.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("../migrations/0004_passes_on_aliases.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("../migrations/0005_sessions_tasks_events.sql"),
    },
];

/// The schema version this build reads and writes: the last migration's number.
pub(crate) const SCHEMA_VERSION: u32 = 5;

/// Adds a session, or for one already stored takes the facts given over those kept and makes it
/// live again; it keeps its first start time. `?9` is the live status's stored name.
pub(crate) const SESSION_UPSERT: &str = "INSERT INTO sessions (session_id, agent, agent_version, \
     model, parent_session_id, cwd, transcript_path, started_at, ended_at, status) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9) \
     ON CONFLICT (session_id) DO UPDATE SET agent = excluded.agent, \
     agent_version = coalesce(excluded.agent_version, agent_version), \
     model = coalesce(excluded.model, model), \
     parent_session_id = coalesce(excluded.parent_session_id, parent_session_id), \
     cwd = coalesce(excluded.cwd, cwd), \
     transcript_path = coalesce(excluded.transcript_path, transcript_path), \
     ended_at = NULL, status = excluded.status";
/// Ends a session: its end time and the ended status's stored name.
pub(crate) const SESSION_END: &str =
    "UPDATE sessions SET ended_at = ?2, status = ?3 WHERE session_id = ?1";
/// A session's row, with its row id first.
pub(crate) const SESSION_BY_ID: &str = "SELECT rowid, session_id, agent, agent_version, model, \
     parent_session_id, cwd, transcript_path, started_at, ended_at, status FROM sessions \
     WHERE session_id = ?1";
/// Adds a task numbered one past every task stored, and returns the number.
pub(crate) const TASK_INSERT: &str = "INSERT INTO tasks (task_id, session_id, seq, started_at, \
     prompt_hash, prompt_excerpt) \
     VALUES (?1, ?2, (SELECT coalesce(max(seq), 0) + 1 FROM tasks), ?3, ?4, ?5) RETURNING seq";
/// Ends a task that is open: its end time and its outcome's stored name.
pub(crate) const TASK_END: &str =
    "UPDATE tasks SET ended_at = ?2, outcome = ?3 WHERE task_id = ?1 AND ended_at IS NULL";
/// A task's row, with its row id first.
pub(crate) const TASK_BY_ID: &str = "SELECT rowid, task_id, session_id, seq, started_at, ended_at, \
     prompt_hash, prompt_excerpt, holdout, outcome FROM tasks WHERE task_id = ?1";
/// A session's latest task that has not ended; the columns are those of [`TASK_BY_ID`].
pub(crate) const TASK_OPEN_OF_SESSION: &str = "SELECT rowid, task_id, session_id, seq, \
     started_at, ended_at, prompt_hash, prompt_excerpt, holdout, outcome FROM tasks \
     WHERE session_id = ?1 AND ended_at IS NULL ORDER BY seq DESC LIMIT 1";
/// Appends an event and returns its number.
pub(crate) const EVENT_INSERT: &str = "INSERT INTO events (task_id, session_id, ts, kind, \
     node_id, path, range_start, range_end, meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
     RETURNING seq";
/// A task's events, in the order they were stored.
pub(crate) const EVENTS_OF_TASK: &str = "SELECT seq, task_id, session_id, ts, kind, node_id, \
     path, range_start, range_end, meta FROM events WHERE task_id = ?1 ORDER BY seq";
/// A session's events by time, then in the order they were stored; the columns are those of
/// [`EVENTS_OF_TASK`].
pub(crate) const EVENTS_OF_SESSION: &str = "SELECT seq, task_id, session_id, ts, kind, node_id, \
     path, range_start, range_end, meta FROM events WHERE session_id = ?1 ORDER BY ts, seq";

/// Events numbered after ?1, in order, at most ?2 of them; the columns are those of
/// [`EVENTS_OF_TASK`].
pub(crate) const EVENTS_AFTER: &str = "SELECT seq, task_id, session_id, ts, kind, node_id, path, \
     range_start, range_end, meta FROM events WHERE seq > ?1 ORDER BY seq LIMIT ?2";

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
/// Removes a `meta` key.
pub(crate) const META_DELETE: &str = "DELETE FROM meta WHERE key = ?1";
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
/// Every file's row, by path.
pub(crate) const FILE_ROWS: &str = "SELECT file_id, path, lang, bytes, loc, content_hash, \
     mtime_ns, flags, indexed_at FROM files ORDER BY path";
/// Sets a file's modification time, found by path.
pub(crate) const FILE_MTIME_SET: &str = "UPDATE files SET mtime_ns = ?2 WHERE path = ?1";
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

/// Every symbol's id, file, file language and name, and whether it is defined directly in its
/// file (in no other definition, its qualified name its name), by id.
pub(crate) const SYMBOL_NAMES: &str = "SELECT s.symbol_id, s.file_id, f.lang, s.name, \
     s.parent_id IS NULL AND s.qual_name = s.name, s.kind \
     FROM symbols AS s JOIN files AS f ON f.file_id = s.file_id ORDER BY s.symbol_id";

/// As [`SYMBOL_NAMES`], for the symbols in the files whose ids a JSON array lists.
pub(crate) const SYMBOL_NAMES_IN_FILES: &str = "SELECT s.symbol_id, s.file_id, f.lang, s.name, \
     s.parent_id IS NULL AND s.qual_name = s.name, s.kind \
     FROM symbols AS s JOIN files AS f ON f.file_id = s.file_id \
     WHERE s.file_id IN (SELECT value FROM json_each(?1)) ORDER BY s.symbol_id";
/// As [`SYMBOL_NAMES`], for the first symbols of one name, by id, in files of the languages a JSON
/// array lists, at most as many as the third parameter.
pub(crate) const SYMBOL_NAMES_NAMED_IN_LANGUAGES: &str = "SELECT s.symbol_id, s.file_id, f.lang, \
     s.name, s.parent_id IS NULL AND s.qual_name = s.name, s.kind \
     FROM symbols AS s JOIN files AS f ON f.file_id = s.file_id \
     WHERE s.name = ?1 AND f.lang IN (SELECT value FROM json_each(?2)) \
     ORDER BY s.symbol_id LIMIT ?3";
/// Adds an unresolved reference and returns its id.
pub(crate) const REF_INSERT: &str = "INSERT INTO refs (src_file_id, src_symbol_id, dst_name, \
     dst_symbol_id, kind, line, resolution, confidence) \
     VALUES (?1, ?2, ?3, NULL, ?4, ?5, NULL, 0.0) RETURNING ref_id";
/// Deletes every reference in a file.
pub(crate) const REFS_DELETE_IN_FILE: &str = "DELETE FROM refs WHERE src_file_id = ?1";
/// Marks every reference to a symbol unresolved; one that went through an import, the
/// resolution given as the second parameter, keeps it.
pub(crate) const REFS_UNRESOLVE_TO: &str = "UPDATE refs SET dst_symbol_id = NULL, \
     resolution = CASE resolution WHEN ?2 THEN ?2 END, confidence = 0.0 \
     WHERE dst_symbol_id = ?1";
/// Sets a reference's target, resolution and confidence, found by id.
pub(crate) const REF_RESOLVE: &str = "UPDATE refs SET dst_symbol_id = ?2, resolution = ?3, \
     confidence = ?4 WHERE ref_id = ?1";
/// A file's references in the order they were written.
pub(crate) const REFS_IN_FILE: &str = "SELECT ref_id, src_file_id, src_symbol_id, dst_name, \
     dst_symbol_id, kind, line, resolution, confidence FROM refs WHERE src_file_id = ?1 \
     ORDER BY ref_id";

/// Every reference's id, file, file language, name, kind, line, target, resolution and
/// confidence, by id.
pub(crate) const REF_NAMES: &str = "SELECT r.ref_id, r.src_file_id, f.lang, r.dst_name, r.kind, \
     r.line, r.dst_symbol_id, r.resolution, r.confidence, r.src_symbol_id FROM refs AS r \
     JOIN files AS f ON f.file_id = r.src_file_id ORDER BY r.ref_id";
/// The files holding a reference of one name that goes through an import, tied or not. The
/// condition on `resolution` is the one `refs_through_imports` is defined with, written the same,
/// as SQLite requires before it uses that index.
pub(crate) const FILES_WITH_REFS_THROUGH_IMPORTS: &str = "SELECT DISTINCT src_file_id FROM refs \
     WHERE dst_name = ?1 AND resolution = 'import_path' ORDER BY src_file_id";
/// As [`REF_NAMES`], for the references of one name that go by it: neither an import's own (the
/// third parameter, its kind) nor going through one (`import_path`, the second parameter).
pub(crate) const REF_NAMES_BY_NAME: &str = "SELECT r.ref_id, r.src_file_id, f.lang, r.dst_name, \
     r.kind, r.line, r.dst_symbol_id, r.resolution, r.confidence, r.src_symbol_id FROM refs AS r \
     JOIN files AS f ON f.file_id = r.src_file_id WHERE r.dst_name = ?1 \
     AND (r.resolution IS NULL OR r.resolution <> ?2) AND r.kind <> ?3 ORDER BY r.ref_id";
/// The files holding a reference tied with one resolution (the second parameter) to a definition
/// in one of the files whose ids a JSON array lists.
pub(crate) const FILES_TIED_INTO: &str = "SELECT DISTINCT r.src_file_id FROM symbols AS s \
     JOIN refs AS r ON r.dst_symbol_id = s.symbol_id \
     WHERE s.file_id IN (SELECT value FROM json_each(?1)) AND r.resolution = ?2 \
     ORDER BY r.src_file_id";
/// The references tied with one resolution, with their files, by id.
pub(crate) const REFS_TIED_AS: &str =
    "SELECT ref_id, src_file_id FROM refs WHERE resolution = ?1 ORDER BY ref_id";
/// As [`REF_NAMES`], for the references in one file.
pub(crate) const REF_NAMES_IN_FILE: &str = "SELECT r.ref_id, r.src_file_id, f.lang, r.dst_name, \
     r.kind, r.line, r.dst_symbol_id, r.resolution, r.confidence, r.src_symbol_id FROM refs AS r \
     JOIN files AS f ON f.file_id = r.src_file_id WHERE r.src_file_id = ?1 ORDER BY r.ref_id";

/// Adds the node for a row, or updates its path, and returns its id.
pub(crate) const NODE_UPSERT: &str = "INSERT INTO nodes (kind, ref_id, path) VALUES (?1, ?2, ?3) \
     ON CONFLICT (kind, ref_id) DO UPDATE SET path = excluded.path RETURNING node_id";
/// The node of a row.
pub(crate) const NODE_OF: &str = "SELECT node_id FROM nodes WHERE kind = ?1 AND ref_id = ?2";
/// Deletes a node.
pub(crate) const NODE_DELETE: &str = "DELETE FROM nodes WHERE node_id = ?1";
// Folder nodes are written with the kind `dir` in the text, as `NodeKind::Dir` names it, so that
// SQLite can use the unique index on folder paths, which holds `dir` nodes only.
/// The node of the folder at a path.
pub(crate) const FOLDER_NODE: &str = "SELECT node_id FROM nodes WHERE kind = 'dir' AND path = ?1";
/// Adds the node of the folder at a path and returns its id.
pub(crate) const FOLDER_NODE_INSERT: &str =
    "INSERT INTO nodes (kind, ref_id, path) VALUES ('dir', NULL, ?1) RETURNING node_id";
/// Every folder node's id and path, by path.
pub(crate) const FOLDER_NODES: &str =
    "SELECT node_id, path, node_id FROM nodes WHERE kind = 'dir' ORDER BY path";
/// The node of the file at a path.
pub(crate) const FILE_NODE_AT: &str = "SELECT n.node_id FROM files AS f JOIN nodes AS n \
     ON n.kind = 'file' AND n.ref_id = f.file_id WHERE f.path = ?1";
/// The id and path of the file a node stands for.
pub(crate) const NODE_FILE: &str = "SELECT f.file_id, f.path FROM nodes AS n JOIN files AS f \
     ON f.file_id = n.ref_id WHERE n.node_id = ?1 AND n.kind = 'file'";
/// The path of the file with a row id.
pub(crate) const FILE_PATH_OF: &str = "SELECT file_id, path FROM files WHERE file_id = ?1";
/// The first files stored between two paths, exclusive, by path, at most as many as the third
/// parameter: those below a folder when the two are the folder's path with `/` and with the next
/// character, `0`, after it.
pub(crate) const FILE_PATHS_BETWEEN: &str =
    "SELECT file_id, path FROM files WHERE path > ?1 AND path < ?2 ORDER BY path LIMIT ?3";
/// Adds one row of what a file passes on.
pub(crate) const PASSES_ON_INSERT: &str =
    "INSERT INTO passes_on (file_id, position, name, taken, path) VALUES (?1, ?2, ?3, ?4, ?5)";
/// Deletes what a file passes on.
pub(crate) const PASSES_ON_DELETE: &str = "DELETE FROM passes_on WHERE file_id = ?1";
/// The names files pass one name (the parameter) on as, other than that name, each with one of its
/// rows' ids. The condition on the two names is `passes_on_aliases`'s own, written the same.
pub(crate) const PASSES_ON_ALIASES: &str = "SELECT name, MIN(rowid) FROM passes_on \
     WHERE taken = ?1 AND name <> taken GROUP BY name ORDER BY name";
/// What a file passes on, in the order it was written.
pub(crate) const PASSES_ON_OF_FILE: &str = "SELECT file_id, position, name, taken, path      FROM passes_on WHERE file_id = ?1 ORDER BY rowid";
/// What every file passes on, by file, in the order it was written.
pub(crate) const PASSES_ON_EVERY: &str =
    "SELECT file_id, position, name, taken, path      FROM passes_on ORDER BY file_id, rowid";
/// Every file's id, path and node, by path.
pub(crate) const FILE_NODES: &str = "SELECT f.file_id, f.path, n.node_id FROM files AS f \
     JOIN nodes AS n ON n.kind = 'file' AND n.ref_id = f.file_id ORDER BY f.path";
/// How many files have a language and none of the flags given, and how many lines they hold.
pub(crate) const SOURCE_SIZE: &str = "SELECT count(*), coalesce(sum(loc), 0) FROM files \
     WHERE lang IS NOT NULL AND flags & ?1 = 0";
/// Every symbol's id and node, by symbol id.
pub(crate) const SYMBOL_NODES: &str =
    "SELECT ref_id, node_id FROM nodes WHERE kind = 'symbol' ORDER BY ref_id";

/// Deletes a node's search row, found by its row id, which is the node's id.
pub(crate) const SEARCH_DELETE: &str = "DELETE FROM search_index WHERE rowid = ?1";
/// Adds a node's search row: its id as row id and as `node_id`, and its tokens.
pub(crate) const SEARCH_INSERT: &str =
    "INSERT INTO search_index (rowid, node_id, tokens) VALUES (?1, ?1, ?2)";
/// A node's search tokens.
pub(crate) const SEARCH_TOKENS: &str = "SELECT tokens FROM search_index WHERE rowid = ?1";

/// Deletes a node's outgoing edges of one type.
pub(crate) const EDGES_DELETE_FROM: &str = "DELETE FROM edges WHERE src = ?1 AND type = ?2";
/// Adds an edge.
pub(crate) const EDGE_INSERT: &str = "INSERT INTO edges (src, dst, type, w, n, last_seq, directed) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";
/// A node's outgoing edges, by type, then target.
pub(crate) const EDGES_FROM: &str = "SELECT src, dst, type, w, n, last_seq, directed FROM edges \
     WHERE src = ?1 ORDER BY type, dst";
/// The nodes the edges of a type into a node start from, in order.
pub(crate) const SOURCES_INTO: &str =
    "SELECT src FROM edges WHERE dst = ?1 AND type = ?2 ORDER BY src";
/// Every node an edge of a type starts from, in order.
pub(crate) const EDGE_SOURCES: &str = "SELECT DISTINCT src FROM edges WHERE type = ?1 ORDER BY src";

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
    type Index = (Vec<String>, bool, bool);

    /// The columns of `table`.
    fn columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<Column>> {
        let mut statement =
            connection.prepare("SELECT name, type, pk FROM pragma_table_info(?1)")?;
        statement
            .query_map([table], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect()
    }

    /// The indexes of `table`, each with its columns, whether it is unique and whether it is
    /// partial, sorted.
    fn indexes(connection: &Connection, table: &str) -> rusqlite::Result<Vec<Index>> {
        let mut statement =
            connection.prepare(r#"SELECT name, "unique", partial FROM pragma_index_list(?1)"#)?;
        let listed: Vec<(String, bool, bool)> = statement
            .query_map([table], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut found = Vec::new();
        for (name, is_unique, is_partial) in listed {
            let mut statement =
                connection.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
            let names: Vec<String> = statement
                .query_map([name], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            found.push((names, is_unique, is_partial));
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
    fn index_list(entries: &[(&[&str], bool, bool)]) -> Vec<Index> {
        let mut list: Vec<Index> = entries
            .iter()
            .map(|(names, is_unique, is_partial)| {
                let names = names.iter().map(|name| (*name).to_owned()).collect();
                (names, *is_unique, *is_partial)
            })
            .collect();
        list.sort();
        list
    }

    /// The columns of every table, typed from the index's table definitions.
    fn expected_columns() -> [(&'static str, Vec<Column>); 8] {
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
            (
                "passes_on",
                column_list(&[
                    ("file_id", "INTEGER", 0),
                    ("position", "INTEGER", 0),
                    ("name", "TEXT", 0),
                    ("taken", "TEXT", 0),
                    ("path", "TEXT", 0),
                ]),
            ),
        ]
    }

    /// The columns of the tables of sessions, tasks and events, typed from their table
    /// definitions.
    fn expected_activity_columns() -> [(&'static str, Vec<Column>); 3] {
        [
            (
                "sessions",
                column_list(&[
                    ("session_id", "TEXT", 1),
                    ("agent", "TEXT", 0),
                    ("agent_version", "TEXT", 0),
                    ("model", "TEXT", 0),
                    ("parent_session_id", "TEXT", 0),
                    ("cwd", "TEXT", 0),
                    ("transcript_path", "TEXT", 0),
                    ("started_at", "INTEGER", 0),
                    ("ended_at", "INTEGER", 0),
                    ("status", "TEXT", 0),
                ]),
            ),
            (
                "tasks",
                column_list(&[
                    ("task_id", "TEXT", 1),
                    ("session_id", "TEXT", 0),
                    ("seq", "INTEGER", 0),
                    ("started_at", "INTEGER", 0),
                    ("ended_at", "INTEGER", 0),
                    ("prompt_hash", "BLOB", 0),
                    ("prompt_excerpt", "TEXT", 0),
                    ("holdout", "INTEGER", 0),
                    ("outcome", "TEXT", 0),
                    ("outcome_evidence", "TEXT", 0),
                ]),
            ),
            (
                "events",
                column_list(&[
                    ("seq", "INTEGER", 1),
                    ("task_id", "TEXT", 0),
                    ("session_id", "TEXT", 0),
                    ("ts", "INTEGER", 0),
                    ("kind", "TEXT", 0),
                    ("node_id", "INTEGER", 0),
                    ("path", "TEXT", 0),
                    ("range_start", "INTEGER", 0),
                    ("range_end", "INTEGER", 0),
                    ("meta", "TEXT", 0),
                ]),
            ),
        ]
    }

    /// The indexes of every table: those of the table definitions, plus `refs.dst_name`, the
    /// references through imports by name, `nodes.path`, the uniqueness of folder paths and the
    /// names passed on as others, the second and the last two partial. Each entry: columns, unique,
    /// partial.
    fn expected_indexes() -> [(&'static str, Vec<Index>); 10] {
        [
            ("meta", index_list(&[(&["key"], true, false)])),
            (
                "files",
                index_list(&[(&["path"], true, false), (&["lang"], false, false)]),
            ),
            (
                "symbols",
                index_list(&[
                    (&["file_id"], false, false),
                    (&["name"], false, false),
                    (&["qual_name"], false, false),
                ]),
            ),
            (
                "refs",
                index_list(&[
                    (&["dst_symbol_id"], false, false),
                    (&["src_file_id"], false, false),
                    (&["dst_name"], false, false),
                    (&["dst_name"], false, true),
                ]),
            ),
            (
                "nodes",
                index_list(&[
                    (&["kind", "ref_id"], true, false),
                    (&["path"], false, false),
                    (&["path"], true, true),
                ]),
            ),
            (
                "edges",
                index_list(&[
                    (&["src", "dst", "type"], true, false),
                    (&["dst", "type"], false, false),
                ]),
            ),
            (
                "passes_on",
                index_list(&[(&["file_id"], false, false), (&["taken"], false, true)]),
            ),
            ("sessions", index_list(&[(&["session_id"], true, false)])),
            (
                "tasks",
                index_list(&[(&["task_id"], true, false), (&["seq"], true, false)]),
            ),
            (
                "events",
                index_list(&[
                    (&["task_id"], false, false),
                    (&["session_id", "ts"], false, false),
                ]),
            ),
        ]
    }

    /// Which files hold a reference of a name that goes through an import is read from the index of
    /// those references alone, as the query plan shows, and the condition both write is the
    /// resolution's stored name.
    #[test]
    fn references_through_imports_are_found_by_their_index()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = Connection::open_in_memory()?;
        for migration in MIGRATIONS {
            connection.execute_batch(migration.sql)?;
        }
        let mut statement = connection
            .prepare(&["EXPLAIN QUERY PLAN ", FILES_WITH_REFS_THROUGH_IMPORTS].concat())?;
        let steps: Vec<String> = statement
            .query_map(["run"], |row| row.get(3))?
            .collect::<rusqlite::Result<_>>()?;
        assert!(
            steps
                .iter()
                .any(|step| step.contains("USING INDEX refs_through_imports")),
            "{steps:?}"
        );
        assert_eq!(cortex_core::Resolution::ImportPath.name(), "import_path");
        Ok(())
    }

    /// The names a name is passed on as are read from the index of those rows alone, as the query
    /// plan shows.
    #[test]
    fn names_passed_on_as_others_are_found_by_their_index() -> Result<(), Box<dyn std::error::Error>>
    {
        let connection = Connection::open_in_memory()?;
        for migration in MIGRATIONS {
            connection.execute_batch(migration.sql)?;
        }
        let mut statement =
            connection.prepare(&["EXPLAIN QUERY PLAN ", PASSES_ON_ALIASES].concat())?;
        let steps: Vec<String> = statement
            .query_map(["x"], |row| row.get(3))?
            .collect::<rusqlite::Result<_>>()?;
        assert!(
            steps
                .iter()
                .any(|step| step.contains("USING INDEX passes_on_aliases")),
            "{steps:?}"
        );
        Ok(())
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
    /// folder paths are unique; one table is added, `passes_on`, what each file passes on of its
    /// imports, indexed by file. Foreign keys refuse references to missing rows, and an edge goes
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
                "events",
                "files",
                "meta",
                "nodes",
                "passes_on",
                "refs",
                "search_index",
                "sessions",
                "symbols",
                "tasks"
            ]
        );
        for (table, expected) in expected_columns()
            .iter()
            .chain(&expected_activity_columns())
        {
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
            foreign_keys(&connection, "passes_on")?,
            [key("files", "file_id", "file_id", "NO ACTION")]
        );
        assert_eq!(
            foreign_keys(&connection, "tasks")?,
            [key("sessions", "session_id", "session_id", "NO ACTION")]
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
