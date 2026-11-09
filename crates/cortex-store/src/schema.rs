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
    Migration {
        version: 6,
        sql: include_str!("../migrations/0006_learning.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("../migrations/0007_usage_service.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("../migrations/0008_usage_response.sql"),
    },
    Migration {
        version: 9,
        sql: include_str!("../migrations/0009_search_terms.sql"),
    },
    Migration {
        version: 10,
        sql: include_str!("../migrations/0010_session_indexes.sql"),
    },
    Migration {
        version: 11,
        sql: include_str!("../migrations/0011_injections.sql"),
    },
    Migration {
        version: 12,
        sql: include_str!("../migrations/0012_cochange_partner_index.sql"),
    },
    Migration {
        version: 13,
        sql: include_str!("../migrations/0013_warnings.sql"),
    },
    Migration {
        version: 14,
        sql: include_str!("../migrations/0014_coordination.sql"),
    },
    Migration {
        version: 15,
        sql: include_str!("../migrations/0015_notice_subjects.sql"),
    },
    Migration {
        version: 16,
        sql: include_str!("../migrations/0016_facts.sql"),
    },
    Migration {
        version: 17,
        sql: include_str!("../migrations/0017_fact_standing.sql"),
    },
    Migration {
        version: 18,
        sql: include_str!("../migrations/0018_root_anchor.sql"),
    },
    Migration {
        version: 19,
        sql: include_str!("../migrations/0019_facts_given.sql"),
    },
];

/// The schema version this build reads and writes: the last migration's number.
pub(crate) const SCHEMA_VERSION: u32 = 19;

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
     prompt_hash, prompt_excerpt, holdout) \
     VALUES (?1, ?2, (SELECT coalesce(max(seq), 0) + 1 FROM tasks), ?3, ?4, ?5, ?6) RETURNING seq";
/// Ends a task that is open: its end time and its outcome's stored name.
pub(crate) const TASK_END: &str =
    "UPDATE tasks SET ended_at = ?2, outcome = ?3 WHERE task_id = ?1 AND ended_at IS NULL";
/// Sets what became of a task that has ended, and the words that say why.
pub(crate) const TASK_OUTCOME: &str = "UPDATE tasks SET outcome = ?2, outcome_evidence = ?3 \
     WHERE task_id = ?1 AND ended_at IS NOT NULL";
/// A task's row, with its row id first.
pub(crate) const TASK_BY_ID: &str = "SELECT rowid, task_id, session_id, seq, started_at, ended_at, \
     prompt_hash, prompt_excerpt, holdout, outcome, outcome_evidence FROM tasks WHERE task_id = ?1";
/// A session's latest task that has not ended; the columns are those of [`TASK_BY_ID`].
pub(crate) const TASK_OPEN_OF_SESSION: &str = "SELECT rowid, task_id, session_id, seq, \
     started_at, ended_at, prompt_hash, prompt_excerpt, holdout, outcome, outcome_evidence \
     FROM tasks WHERE session_id = ?1 AND ended_at IS NULL ORDER BY seq DESC LIMIT 1";
/// Whether session ?1 has a task other than ?2.
pub(crate) const SESSION_HAS_OTHER_TASK: &str =
    "SELECT EXISTS (SELECT 1 FROM tasks WHERE session_id = ?1 AND task_id <> ?2)";
/// The tasks not ended whose last sign of work, the latest of their start, their own events and
/// their session's events, is at or before ?1: task id, session id and that time, oldest first.
pub(crate) const QUIET_OPEN_TASKS: &str = "SELECT task_id, session_id, last FROM (SELECT \
     t.task_id, t.session_id, max(t.started_at, \
     coalesce((SELECT max(e.ts) FROM events e WHERE e.task_id = t.task_id), t.started_at), \
     coalesce((SELECT max(e.ts) FROM events e WHERE e.session_id = t.session_id), t.started_at)) \
     AS last FROM tasks t WHERE t.ended_at IS NULL) WHERE last <= ?1 ORDER BY last, task_id";
/// The sessions that started at or after ?1, by start and then id; the columns are those of
/// [`SESSION_BY_ID`].
pub(crate) const SESSIONS_STARTED_SINCE: &str = "SELECT rowid, session_id, agent, agent_version, \
     model, parent_session_id, cwd, transcript_path, started_at, ended_at, status FROM sessions \
     WHERE started_at >= ?1 ORDER BY started_at, session_id";
/// The sessions ?1 started, by start and then id; the columns are those of [`SESSION_BY_ID`].
pub(crate) const SESSIONS_STARTED_BY: &str = "SELECT rowid, session_id, agent, agent_version, \
     model, parent_session_id, cwd, transcript_path, started_at, ended_at, status FROM sessions \
     WHERE parent_session_id = ?1 ORDER BY started_at, session_id";
/// A session's tasks in order; the columns are those of [`TASK_BY_ID`].
pub(crate) const TASKS_OF_SESSION: &str = "SELECT rowid, task_id, session_id, seq, started_at, \
     ended_at, prompt_hash, prompt_excerpt, holdout, outcome, outcome_evidence FROM tasks \
     WHERE session_id = ?1 ORDER BY seq";
/// How many tool calls each task a JSON array lists made, with the first row of each. A report is
/// stored as one row, or as one row per stretch of its file when it names several, each with its
/// lines: rows without lines are a report each, and rows with lines count once for each session,
/// time, kind and path they share. Rows of another source than a hook, the watcher's edits, are
/// no calls.
pub(crate) const TOOL_CALLS_OF_TASKS: &str = "SELECT task_id, count(*), min(first) FROM \
     (SELECT task_id, min(rowid) AS first FROM events \
     WHERE task_id IN (SELECT value FROM json_each(?1)) \
     AND kind IN ('read', 'search', 'edit', 'shell', 'tool') \
     AND coalesce(json_extract(meta, '$.source'), 'hook') = 'hook' \
     GROUP BY task_id, session_id, ts, kind, path, \
     CASE WHEN range_start IS NULL THEN rowid END) GROUP BY task_id ORDER BY task_id";
/// How many tool calls session ?1 made while it had no task, counted as [`TOOL_CALLS_OF_TASKS`]
/// counts them.
pub(crate) const TOOL_CALLS_OUTSIDE_TASKS: &str = "SELECT count(*) FROM (SELECT 1 FROM events \
     WHERE session_id = ?1 AND task_id IS NULL \
     AND kind IN ('read', 'search', 'edit', 'shell', 'tool') \
     AND coalesce(json_extract(meta, '$.source'), 'hook') = 'hook' \
     GROUP BY ts, kind, path, CASE WHEN range_start IS NULL THEN rowid END)";
/// How many times session ?1 compacted its conversation; its rows are found by their index.
pub(crate) const COMPACTIONS_OF_SESSION: &str =
    "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'compact'";
/// How many of the repository's tasks were held out from Cortex, and how many were not, among
/// those decided.
pub(crate) const HOLDOUT_ARMS: &str = "SELECT coalesce(sum(holdout = 1), 0), \
     coalesce(sum(holdout = 0), 0) FROM tasks";

/// Appends an event and returns its number.
pub(crate) const EVENT_INSERT: &str = "INSERT INTO events (task_id, session_id, ts, kind, \
     node_id, path, range_start, range_end, meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
     RETURNING seq";
/// A task's events, in the order they were stored, at most ?2 of them.
pub(crate) const EVENTS_OF_TASK: &str = "SELECT seq, task_id, session_id, ts, kind, node_id, \
     path, range_start, range_end, meta FROM events WHERE task_id = ?1 ORDER BY seq LIMIT ?2";
/// A session's events by time, then in the order they were stored; the columns are those of
/// [`EVENTS_OF_TASK`].
pub(crate) const EVENTS_OF_SESSION: &str = "SELECT seq, task_id, session_id, ts, kind, node_id, \
     path, range_start, range_end, meta FROM events WHERE session_id = ?1 ORDER BY ts, seq";
/// A session's reads of one file's node and its compactions, the latest first, at most ?3 of them;
/// the columns are those of [`EVENTS_OF_TASK`]. The session's rows are found by their index.
pub(crate) const READS_IN_SESSION: &str = "SELECT seq, task_id, session_id, ts, kind, node_id, \
     path, range_start, range_end, meta FROM events WHERE session_id = ?1 \
     AND ((kind = 'read' AND node_id = ?2) OR kind = 'compact') \
     ORDER BY ts DESC, seq DESC LIMIT ?3";

/// A task's latest events, the last stored first, at most ?2 of them; the columns are those of
/// [`EVENTS_OF_TASK`].
pub(crate) const LATEST_EVENTS_OF_TASK: &str = "SELECT seq, task_id, session_id, ts, kind, \
     node_id, path, range_start, range_end, meta FROM events WHERE task_id = ?1 \
     ORDER BY seq DESC LIMIT ?2";

/// The edits of the tasks of session ?1, its subagents' edits included, newest first by time and
/// then by number, at most ?2 of them; the columns are those of [`EVENTS_OF_TASK`]. Only the edit
/// rows of the session's tasks are read (the index of edits), and only those are sorted.
pub(crate) const EDITS_OF_SESSION: &str = "SELECT seq, task_id, session_id, ts, kind, node_id, \
     path, range_start, range_end, meta FROM events \
     WHERE task_id IN (SELECT task_id FROM tasks WHERE session_id = ?1) AND kind = 'edit' \
     ORDER BY ts DESC, seq DESC LIMIT ?2";
/// The reads and compactions session ?1 itself reported, newest first by time and then by
/// number, at most ?2 of them; the columns are those of [`EVENTS_OF_TASK`].
pub(crate) const READS_OF_SESSION: &str = "SELECT seq, task_id, session_id, ts, kind, node_id, \
     path, range_start, range_end, meta FROM events \
     WHERE session_id = ?1 AND kind IN ('read', 'compact') ORDER BY ts DESC, seq DESC LIMIT ?2";

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
/// Sets a file's modification time and when the index last read it, found by path.
pub(crate) const FILE_TOUCH: &str =
    "UPDATE files SET mtime_ns = ?2, indexed_at = ?3 WHERE path = ?1";
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
/// The node and lines of each symbol in a file, in the order they appear.
// A `CROSS JOIN` keeps SQLite from starting at the nodes: by their kind alone that is every symbol
// node of the repository, where the file's or the name's own index finds a handful.
pub(crate) const SYMBOL_SPANS_IN_FILE: &str = "SELECT n.node_id, s.start_line, s.end_line, \
     s.symbol_id, s.parent_id, s.kind, s.name \
     FROM symbols AS s CROSS JOIN nodes AS n ON n.kind = 'symbol' AND n.ref_id = s.symbol_id \
     WHERE s.file_id = ?1 ORDER BY s.start_byte, s.symbol_id";
/// The files the file nodes in a JSON array stand for, by node; the columns are those of
/// [`FILE_BY_PATH`] and then the node.
pub(crate) const FILES_OF_NODES: &str = "SELECT f.file_id, f.path, f.lang, f.bytes, f.loc, \
     f.content_hash, f.mtime_ns, f.flags, f.indexed_at, n.node_id \
     FROM (SELECT DISTINCT value FROM json_each(?1)) AS j \
     CROSS JOIN nodes AS n ON n.node_id = j.value CROSS JOIN files AS f ON f.file_id = n.ref_id \
     WHERE n.kind = 'file' ORDER BY n.node_id";

/// The symbols with a name, oldest first.
pub(crate) const SYMBOLS_NAMED: &str =
    concat!(symbol_columns!(), "WHERE name = ?1 ORDER BY symbol_id");

/// The symbols with a name or a qualified name, oldest first.
pub(crate) const SYMBOLS_CALLED: &str = concat!(
    symbol_columns!(),
    "WHERE qual_name = ?1 OR name = ?1 ORDER BY symbol_id"
);

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
/// The references tied to one symbol, by file and line, read as [`REFS_IN_FILE`] reads them.
pub(crate) const REFS_TIED_TO: &str = "SELECT ref_id, src_file_id, src_symbol_id, dst_name, \
     dst_symbol_id, kind, line, resolution, confidence FROM refs WHERE dst_symbol_id = ?1 \
     ORDER BY src_file_id, line, ref_id";

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
/// The id, path and node of every file that counts for co-change: neither generated nor vendored
/// (the flags given), by path.
pub(crate) const COCHANGE_FILES: &str = "SELECT f.file_id, f.path, n.node_id FROM files AS f \
     JOIN nodes AS n ON n.kind = 'file' AND n.ref_id = f.file_id WHERE f.flags & ?1 = 0 \
     ORDER BY f.path";
/// Deletes every file's commit counts.
pub(crate) const FILE_COMMITS_DELETE_ALL: &str = "DELETE FROM file_commits";
/// Deletes every pair's co-change counts.
pub(crate) const COCHANGE_STATS_DELETE_ALL: &str = "DELETE FROM cochange_stats";
/// Adds a file's commit counts to those stored, or stores them.
pub(crate) const FILE_COMMITS_ADD: &str = "INSERT INTO file_commits (file_id, count, weighted) \
     VALUES (?1, ?2, ?3) ON CONFLICT (file_id) DO UPDATE SET count = count + excluded.count, \
     weighted = weighted + excluded.weighted";
/// Adds a pair's co-change counts to those stored, or stores them; the newest shared commit is
/// the newer of the two.
pub(crate) const COCHANGE_STATS_ADD: &str = "INSERT INTO cochange_stats \
     (file_a, file_b, count, weighted, last_commit_seq) VALUES (?1, ?2, ?3, ?4, ?5) \
     ON CONFLICT (file_a, file_b) DO UPDATE SET count = count + excluded.count, \
     weighted = weighted + excluded.weighted, \
     last_commit_seq = max(last_commit_seq, excluded.last_commit_seq)";
/// The commit counts of the files whose ids are in a JSON array, by file id.
pub(crate) const FILE_COMMITS_OF: &str = "SELECT file_id, count, weighted FROM file_commits \
     WHERE file_id IN (SELECT value FROM json_each(?1)) ORDER BY file_id";
/// The co-change counts of every pair holding a file whose id is in a JSON array, by pair.
pub(crate) const COCHANGE_STATS_OF: &str = "SELECT file_a, file_b, count, weighted, \
     last_commit_seq FROM cochange_stats WHERE file_a IN (SELECT value FROM json_each(?1)) \
     UNION SELECT file_a, file_b, count, weighted, last_commit_seq FROM cochange_stats \
     WHERE file_b IN (SELECT value FROM json_each(?1)) ORDER BY file_a, file_b";
/// A value of the co-change miner's state.
pub(crate) const GIT_STATE_GET: &str = "SELECT value FROM git_state WHERE key = ?1";
/// Sets a value of the co-change miner's state.
pub(crate) const GIT_STATE_SET: &str = "INSERT INTO git_state (key, value) VALUES (?1, ?2) \
     ON CONFLICT (key) DO UPDATE SET value = excluded.value";
/// Deletes a value of the co-change miner's state.
pub(crate) const GIT_STATE_DELETE: &str = "DELETE FROM git_state WHERE key = ?1";

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
/// The search tokens of the nodes in a JSON array of ids that have a search row, by node.
pub(crate) const SEARCH_TOKENS_OF: &str = "SELECT rowid, tokens FROM search_index \
     WHERE rowid IN (SELECT value FROM json_each(?1)) ORDER BY rowid";

/// Every node, by node, with its kind; for a symbol, the node of the file it is defined in; its
/// interest, when it has any; and for a file, its flags.
pub(crate) const GRAPH_NODES: &str = "SELECT n.node_id, n.kind, f.node_id, i.score, i.last_seq, \
     i.tier, ff.flags FROM nodes AS n \
     LEFT JOIN symbols AS s ON n.kind = 'symbol' AND s.symbol_id = n.ref_id \
     LEFT JOIN nodes AS f ON f.kind = 'file' AND f.ref_id = s.file_id \
     LEFT JOIN interest AS i ON i.node_id = n.node_id \
     LEFT JOIN files AS ff ON n.kind = 'file' AND ff.file_id = n.ref_id ORDER BY n.node_id";
/// Every edge, in the order of its key: source, target, type.
pub(crate) const ALL_EDGES: &str =
    "SELECT src, dst, type, w, n, last_seq, directed FROM edges ORDER BY src, dst, type";
/// The number of the latest task, 0 when there is none.
pub(crate) const LATEST_TASK_SEQ: &str = "SELECT coalesce(max(seq), 0) FROM tasks";

/// How many search rows hold a word, from the index's vocabulary.
pub(crate) const SEARCH_TERM_ROWS: &str = "SELECT doc FROM search_terms WHERE term = ?1";
/// The best search rows for a full-text expression, best first and then by node, at most ?2, each
/// with its BM25 score made positive: the index gives the best match the lowest.
pub(crate) const SEARCH_MATCH: &str = "SELECT rowid, -rank FROM search_index \
     WHERE search_index MATCH ?1 ORDER BY rank, rowid LIMIT ?2";
/// The files whose path is ?1 or matches ?2, a `LIKE` pattern escaped with a backslash, by path,
/// with their ids and nodes.
pub(crate) const FILES_NAMED: &str = "SELECT f.path, f.file_id, n.node_id FROM files AS f \
     JOIN nodes AS n ON n.kind = 'file' AND n.ref_id = f.file_id \
     WHERE f.path = ?1 OR f.path LIKE ?2 ESCAPE '\\' ORDER BY f.path";
/// The nodes of the symbols named ?1, by node, at most ?2.
pub(crate) const SYMBOL_NODES_NAMED: &str = "SELECT n.node_id FROM symbols AS s \
     CROSS JOIN nodes AS n ON n.kind = 'symbol' AND n.ref_id = s.symbol_id \
     WHERE s.name = ?1 ORDER BY n.node_id LIMIT ?2";

/// Deletes a node's outgoing edges of one type.
pub(crate) const EDGES_DELETE_FROM: &str = "DELETE FROM edges WHERE src = ?1 AND type = ?2";
/// Deletes every edge of one type.
pub(crate) const EDGES_DELETE_OF_TYPE: &str = "DELETE FROM edges WHERE type = ?1";
/// Adds an edge.
pub(crate) const EDGE_INSERT: &str = "INSERT INTO edges (src, dst, type, w, n, last_seq, directed) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";
/// A node's outgoing edges, by type, then target.
pub(crate) const EDGES_FROM: &str = "SELECT src, dst, type, w, n, last_seq, directed FROM edges \
     WHERE src = ?1 ORDER BY type, dst";
/// The edges of a type running between two of the nodes whose ids a JSON array lists, by source
/// and then target.
pub(crate) const EDGES_AMONG: &str = "SELECT src, dst, type, w, n, last_seq, directed FROM edges \
     WHERE type = ?1 AND src IN (SELECT value FROM json_each(?2)) \
     AND dst IN (SELECT value FROM json_each(?2)) ORDER BY src, dst";
/// Sets an edge's weight, count and task, adding it if it is not there.
pub(crate) const EDGE_SET: &str = "INSERT INTO edges (src, dst, type, w, n, last_seq, directed) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (src, dst, type) DO UPDATE SET \
     w = excluded.w, n = excluded.n, last_seq = excluded.last_seq, directed = excluded.directed";
/// The nodes the edges of a type into a node start from, in order.
pub(crate) const SOURCES_INTO: &str =
    "SELECT src FROM edges WHERE dst = ?1 AND type = ?2 ORDER BY src";
/// The files holding a test that have a structural edge into one node, each with the edge's
/// weight, by node. The edges into the node are found by their index; each file's tests by its
/// symbols' index.
pub(crate) const TEST_FILES_INTO: &str = "SELECT e.src, e.w FROM edges AS e \
     CROSS JOIN nodes AS n ON n.node_id = e.src \
     WHERE e.dst = ?1 AND e.type = ?2 AND n.kind = 'file' AND EXISTS \
     (SELECT 1 FROM symbols AS s WHERE s.file_id = n.ref_id AND s.kind = 'test') \
     ORDER BY e.src";
/// Every node an edge of a type starts from, in order.
pub(crate) const EDGE_SOURCES: &str = "SELECT DISTINCT src FROM edges WHERE type = ?1 ORDER BY src";

/// The interest of the nodes whose ids a JSON array lists, by node.
pub(crate) const INTEREST_OF: &str = "SELECT node_id, score, last_seq, tier FROM interest \
     WHERE node_id IN (SELECT value FROM json_each(?1)) ORDER BY node_id";
/// Every file node whose stored score is at least the first parameter, by node.
pub(crate) const FILE_INTEREST_FROM: &str = "SELECT i.node_id, i.score, i.last_seq, i.tier \
     FROM interest AS i JOIN nodes AS n ON n.node_id = i.node_id \
     WHERE n.kind = 'file' AND i.score >= ?1 ORDER BY i.node_id";
/// Writes a node's interest, replacing what it had.
pub(crate) const INTEREST_REPLACE: &str = "INSERT INTO interest (node_id, score, last_seq, tier) \
     VALUES (?1, ?2, ?3, ?4) ON CONFLICT (node_id) DO UPDATE SET score = excluded.score, \
     last_seq = excluded.last_seq, tier = excluded.tier";

/// Writes one response's usage, replacing what its session and turn had.
pub(crate) const USAGE_REPLACE: &str = "INSERT INTO usage \
     (session_id, turn, ts, model, input, cache_write, cache_write_1h, cache_read, output, \
     reasoning, service_tier, speed, inference_geo, response) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
     ON CONFLICT (session_id, turn) DO UPDATE SET \
     ts = excluded.ts, model = excluded.model, input = excluded.input, \
     cache_write = excluded.cache_write, cache_write_1h = excluded.cache_write_1h, \
     cache_read = excluded.cache_read, output = excluded.output, \
     reasoning = excluded.reasoning, service_tier = excluded.service_tier, \
     speed = excluded.speed, inference_geo = excluded.inference_geo, \
     response = excluded.response";
/// Removes the usage rows of a session numbered past a turn.
pub(crate) const USAGE_FORGET_AFTER: &str = "DELETE FROM usage WHERE session_id = ?1 AND turn > ?2";
/// A session's usage, by turn, without a response another session that started earlier reports
/// too (of two that started together, the one with the smaller id keeps it).
pub(crate) const USAGE_OF_SESSION: &str = "SELECT u.rowid, u.session_id, u.turn, u.ts, \
     u.input, u.cache_write, u.cache_write_1h, u.cache_read, u.output, u.reasoning, u.model, \
     u.service_tier, u.speed, u.inference_geo, u.response \
     FROM usage AS u JOIN sessions AS s ON s.session_id = u.session_id \
     WHERE u.session_id = ?1 AND NOT EXISTS (SELECT 1 FROM usage AS c \
     JOIN sessions AS cs ON cs.session_id = c.session_id \
     WHERE c.response = u.response AND c.session_id <> u.session_id \
     AND (cs.started_at < s.started_at \
     OR (cs.started_at = s.started_at AND c.session_id < u.session_id))) \
     ORDER BY u.turn";

/// Writes a task's record, replacing the one it had.
pub(crate) const EPISODE_REPLACE: &str = "INSERT INTO episodes \
     (task_id, localize, change, verify, pitfalls, outcome, created_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (task_id) DO UPDATE SET \
     localize = excluded.localize, change = excluded.change, verify = excluded.verify, \
     pitfalls = excluded.pitfalls, outcome = excluded.outcome, created_at = excluded.created_at";
/// A task's record.
pub(crate) const EPISODE_OF_TASK: &str = "SELECT localize, change, verify, pitfalls, outcome, \
     created_at FROM episodes WHERE task_id = ?1";

/// Deletes every cluster of tasks.
pub(crate) const ARCHETYPES_DELETE_ALL: &str = "DELETE FROM archetypes";
/// Adds one cluster of tasks.
pub(crate) const ARCHETYPE_INSERT: &str = "INSERT INTO archetypes \
     (seed_terms, files, episode_count, updated_at) VALUES (?1, ?2, ?3, ?4)";
/// Every cluster of tasks, oldest first.
pub(crate) const ARCHETYPES: &str = "SELECT seed_terms, files, episode_count, updated_at \
     FROM archetypes ORDER BY archetype_id";
/// The records of the newest tasks, at most as many as the first parameter, oldest first.
pub(crate) const EPISODES: &str = "SELECT task_id, localize, change, verify, pitfalls, \
     outcome, created_at FROM (SELECT task_id, localize, change, verify, pitfalls, outcome, \
     created_at FROM episodes ORDER BY task_id DESC LIMIT ?1) ORDER BY task_id";

/// Row counts for status.
pub(crate) const COUNTS: &str = "SELECT (SELECT count(*) FROM files), \
     (SELECT count(*) FROM symbols), (SELECT count(*) FROM refs), \
     (SELECT count(*) FROM refs WHERE dst_symbol_id IS NOT NULL), \
     (SELECT count(*) FROM nodes), (SELECT count(*) FROM edges)";

/// Records an injection, returning its row id.
pub(crate) const INJECTION_INSERT: &str = "INSERT INTO injections \
    (task_id, session_id, hook, ts, items, tokens_est, confidence, gated_out, holdout, \
    rendered_hash, features) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

/// The labels of the latest injections that reached the agent and are labelled, the newest first:
/// neither kept back nor held out, with their text's hash, as `InjectionRecord::was_delivered`
/// tells one (a primer is never labelled, so none is among them).
pub(crate) const DELIVERED_LABELS: &str = "SELECT label FROM injections \
    WHERE gated_out = 0 AND holdout = 0 AND rendered_hash IS NOT NULL AND label IS NOT NULL \
    ORDER BY injection_id DESC LIMIT ?1";

/// How many injections that reached the agent are labelled, told as [`DELIVERED_LABELS`] tells
/// them.
pub(crate) const COUNT_DELIVERED_LABELLED: &str = "SELECT count(*) FROM injections \
    WHERE gated_out = 0 AND holdout = 0 AND rendered_hash IS NOT NULL AND label IS NOT NULL";

/// How many injections that reached the agent are labelled, and how many of them are labelled
/// used, told as [`DELIVERED_LABELS`] tells them.
pub(crate) const DELIVERED_PRECISION: &str = "SELECT count(*), coalesce(sum(label), 0) \
    FROM injections \
    WHERE gated_out = 0 AND holdout = 0 AND rendered_hash IS NOT NULL AND label IS NOT NULL";

/// Sets what the end of its task found of one injection: its used count and its label.
pub(crate) const INJECTION_LABEL: &str =
    "UPDATE injections SET used_count = ?2, label = ?3 WHERE injection_id = ?1";

/// The injections of one task, in the order they were recorded, read back as
/// [`crate::injections`] reads them.
pub(crate) const INJECTIONS_OF_TASK: &str = "SELECT injection_id, task_id, session_id, hook, ts, \
    items, tokens_est, confidence, gated_out, holdout, rendered_hash, used_count, label, features \
    FROM injections WHERE task_id = ?1 ORDER BY injection_id";

/// The injections of one session, in the order they were recorded, read back as
/// [`crate::injections`] reads them.
pub(crate) const INJECTIONS_OF_SESSION: &str = "SELECT injection_id, task_id, session_id, hook, \
    ts, items, tokens_est, confidence, gated_out, holdout, rendered_hash, used_count, label, \
    features FROM injections WHERE session_id = ?1 ORDER BY injection_id";

/// The work orders that reached session ?1's agent after ?2, the latest first, at most ?3: rows of
/// prompts' retrievals (which alone carry the gate's features; a prompt's coordination notices do
/// not) neither kept back nor held out, with a text's hash; the columns are those of
/// [`INJECTIONS_OF_SESSION`].
pub(crate) const WORK_ORDERS_SINCE: &str = "SELECT injection_id, task_id, session_id, hook, ts, \
    items, tokens_est, confidence, gated_out, holdout, rendered_hash, used_count, label, features \
    FROM injections WHERE session_id = ?1 AND ts > ?2 AND hook = 'prompt' AND gated_out = 0 \
    AND holdout = 0 AND rendered_hash IS NOT NULL AND features IS NOT NULL \
    ORDER BY ts DESC, injection_id DESC LIMIT ?3";

/// Records one file a warning named, returning its row id.
pub(crate) const WARNING_INSERT: &str = "INSERT INTO warnings \
    (task_id, ts, kind, src_node, dst_node, strength) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

/// Sets whether one warning was resolved.
pub(crate) const WARNING_RESOLVE: &str = "UPDATE warnings SET resolved = ?2 WHERE warning_id = ?1";

/// Keeps a session's position on a node, in place of the one kept before.
pub(crate) const POSITION_KEEP: &str = "INSERT OR REPLACE INTO positions \
     (session_id, node_id, mode, since, last_ts) VALUES (?1, ?2, ?3, ?4, ?5)";
/// Session ?1's position on node ?2, if one is kept, with when its task ended, as for
/// [`POSITIONS_SINCE`].
pub(crate) const POSITION_OF: &str = "SELECT p.mode, p.since, p.last_ts, \
     (SELECT CASE WHEN t.ended_at >= p.last_ts THEN t.ended_at END FROM tasks t \
     WHERE t.session_id = p.session_id AND t.started_at <= p.last_ts \
     ORDER BY t.started_at DESC LIMIT 1) \
     FROM positions p WHERE p.session_id = ?1 AND p.node_id = ?2";
/// The positions last taken at ?1 or later, with when the task the session had then ended: the
/// latest task of the session started by then, when it ended at the position's last time or
/// later. A position taken after that task ended, with no prompt since, has no task end.
pub(crate) const POSITIONS_SINCE: &str = "SELECT p.session_id, p.node_id, p.mode, p.since, \
     p.last_ts, (SELECT CASE WHEN t.ended_at >= p.last_ts THEN t.ended_at END FROM tasks t \
     WHERE t.session_id = p.session_id AND t.started_at <= p.last_ts \
     ORDER BY t.started_at DESC LIMIT 1) \
     FROM positions p WHERE p.last_ts >= ?1 ORDER BY p.session_id, p.node_id";
/// Each session in the JSON array ?1 that is stored: its agent, its parent, when it ended and its
/// last event.
pub(crate) const SESSIONS_SEEN: &str = "SELECT s.session_id, s.agent, s.parent_session_id, \
     s.ended_at, (SELECT max(e.ts) FROM events e WHERE e.session_id = s.session_id), s.status \
     FROM sessions s WHERE s.session_id IN (SELECT value FROM json_each(?1)) ORDER BY s.session_id";
/// The heaviest edge of each type between the file node ?1, with its symbols, and each file node
/// in the JSON array ?2, with theirs: edges of type ?3 either way, and of type ?4 from ?1's side
/// only. A file's symbols carry its path, which is how they are found.
pub(crate) const COUPLINGS: &str = "WITH held AS (SELECT n.node_id, n.path FROM \
     (SELECT DISTINCT value FROM json_each(?2)) AS j JOIN nodes AS n ON n.node_id = j.value \
     WHERE n.kind = 'file'), \
     mine AS (SELECT node_id FROM nodes WHERE kind != 'dir' AND path = \
     (SELECT path FROM nodes WHERE node_id = ?1 AND kind = 'file')), \
     linked AS (SELECT o.path, e.type, e.w FROM mine AS m JOIN edges AS e ON e.src = m.node_id \
     JOIN nodes AS o ON o.node_id = e.dst WHERE e.type IN (?3, ?4) \
     UNION ALL SELECT o.path, e.type, e.w FROM mine AS m JOIN edges AS e ON e.dst = m.node_id \
     JOIN nodes AS o ON o.node_id = e.src WHERE e.type = ?3) \
     SELECT h.node_id, l.type, max(l.w) FROM linked AS l JOIN held AS h ON h.path = l.path \
     GROUP BY h.node_id, l.type ORDER BY h.node_id, l.type";
/// Queues notice ?2, about ?4, for session ?1 at ?3.
pub(crate) const NOTICE_QUEUE: &str =
    "INSERT INTO notices (to_session, text, created_at, about) VALUES (?1, ?2, ?3, ?4)";
/// Records notice ?2, about ?4, as told to session ?1 at ?3, delivered as it was made.
pub(crate) const NOTICE_TOLD: &str = "INSERT INTO notices (to_session, text, created_at, \
     delivered_at, about) VALUES (?1, ?2, ?3, ?3, ?4)";
/// The notices session ?1 was given or sent at ?2 or later, oldest first.
pub(crate) const NOTICES_SINCE: &str = "SELECT notice_id, about, text, created_at FROM notices \
     WHERE to_session = ?1 AND created_at >= ?2 ORDER BY notice_id";
/// The notices waiting for session ?1 made at ?2 or later, oldest first.
pub(crate) const NOTICES_WAITING: &str = "SELECT notice_id, about, text, created_at \
     FROM notices WHERE to_session = ?1 AND created_at >= ?2 AND delivered_at IS NULL \
     ORDER BY notice_id";
/// Leaves the handoff of session ?1 for task ?2: its files ?3, its open warnings ?4, its last
/// failure ?5, at ?6.
pub(crate) const HANDOFF_LEAVE: &str = "INSERT INTO handoffs (from_session, task_id, \
     working_set, open_warnings, last_failure, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
/// Closes the open handoffs session ?1 left, as replaced by the one it leaves now.
pub(crate) const HANDOFFS_REPLACE_OF_SESSION: &str =
    "UPDATE handoffs SET consumed_by = ?1 WHERE from_session = ?1 AND consumed_by IS NULL";
/// Closes handoff ?1, as picked up by session ?2, unless it was already closed.
pub(crate) const HANDOFF_CLOSE: &str =
    "UPDATE handoffs SET consumed_by = ?2 WHERE handoff_id = ?1 AND consumed_by IS NULL";
/// The open handoffs left at ?1 or later, the newest first.
pub(crate) const HANDOFFS_OPEN_SINCE: &str = "SELECT handoff_id, from_session, task_id, \
     working_set, open_warnings, last_failure, created_at FROM handoffs \
     WHERE consumed_by IS NULL AND created_at >= ?1 ORDER BY created_at DESC, handoff_id DESC";
/// The open handoffs session ?1 left, the latest first.
pub(crate) const HANDOFFS_OPEN_OF_SESSION: &str = "SELECT handoff_id, from_session, task_id, \
     working_set, open_warnings, last_failure, created_at FROM handoffs \
     WHERE from_session = ?1 AND consumed_by IS NULL ORDER BY handoff_id DESC";
/// Marks notice ?1 delivered at ?2, unless it was already.
pub(crate) const NOTICE_DELIVER: &str =
    "UPDATE notices SET delivered_at = ?2 WHERE notice_id = ?1 AND delivered_at IS NULL";

/// Adds a fact: its text ?1, kind ?2, source ?3, status ?4, made at ?5 (its status set and
/// confirmed then too), first asserted in task ?6.
pub(crate) const FACT_ADD: &str = "INSERT INTO facts (text, kind, source, status, status_at, \
     asserted_in, created_at, last_verified_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?5, ?5)";
/// Anchors fact ?1 to node ?2, whose hash was ?3 when the fact was made.
pub(crate) const FACT_ANCHOR_ADD: &str =
    "INSERT INTO fact_anchors (fact_id, node_id, anchor_hash) VALUES (?1, ?2, ?3)";
/// Sets fact ?1's status to ?2 at ?3; one made active counts as confirmed then (?4 is 1). Made
/// suspect (?5 names the status), it keeps the status it held before, which it held already when
/// it was suspect; any other status forgets it.
pub(crate) const FACT_SET_STATUS: &str = "UPDATE facts SET status = ?2, status_at = ?3, \
     last_verified_at = CASE WHEN ?4 THEN ?3 ELSE last_verified_at END, \
     before_suspect = CASE WHEN ?2 = ?5 THEN \
     (CASE WHEN status = ?5 THEN before_suspect ELSE status END) ELSE NULL END \
     WHERE fact_id = ?1";
/// Records task ?2 as the one that first asserted fact ?1, unless one already is.
pub(crate) const FACT_NOTE_TASK: &str =
    "UPDATE facts SET asserted_in = ?2 WHERE fact_id = ?1 AND asserted_in IS NULL";
/// Gives fact ?1's anchor on node ?2 the hash ?3.
pub(crate) const FACT_ANCHOR_RESTAMP: &str =
    "UPDATE fact_anchors SET anchor_hash = ?3 WHERE fact_id = ?1 AND node_id = ?2";
/// Marks fact ?1 superseded by fact ?2.
pub(crate) const FACT_SUPERSEDE: &str =
    "UPDATE facts SET superseded_by = ?2 WHERE fact_id = ?1 AND superseded_by IS NULL";
/// Records that task ?1 was given each fact in the JSON list ?2, once each.
pub(crate) const FACTS_GIVEN_ADD: &str = "INSERT OR IGNORE INTO facts_given (task_id, fact_id) \
     SELECT ?1, value FROM json_each(?2)";
/// The facts task ?1 was given, in id order.
pub(crate) const FACTS_GIVEN_IN_TASK: &str =
    "SELECT fact_id FROM facts_given WHERE task_id = ?1 ORDER BY fact_id";
/// Counts one more use of each fact in the JSON list ?1.
pub(crate) const FACT_COUNT_USES: &str =
    "UPDATE facts SET uses = uses + 1 WHERE fact_id IN (SELECT value FROM json_each(?1))";
/// A fact's columns by its id.
pub(crate) const FACT_BY_ID: &str = "SELECT fact_id, text, kind, source, status, status_at, \
     asserted_in, uses, created_at, last_verified_at, superseded_by, before_suspect FROM facts \
     WHERE fact_id = ?1";
/// The current facts (none supersedes them) of status ?1, oldest first.
pub(crate) const FACTS_CURRENT_WITH_STATUS: &str = "SELECT fact_id, text, kind, source, status, \
     status_at, asserted_in, uses, created_at, last_verified_at, superseded_by, before_suspect \
     FROM facts \
     WHERE status = ?1 AND superseded_by IS NULL ORDER BY fact_id";
/// The current facts of status ?1 or ?2 with an anchor whose node's hash is not the one kept (or
/// that has none now: the node is gone, or its contents are no longer read), in id order. Each
/// anchor's hash now is looked up by its node: joined whole, the view would be built over every
/// node of the repository.
pub(crate) const FACTS_WITH_CHANGED_ANCHORS: &str = "SELECT DISTINCT a.fact_id \
     FROM fact_anchors AS a JOIN facts AS f ON f.fact_id = a.fact_id \
     WHERE f.status IN (?1, ?2) AND f.superseded_by IS NULL \
     AND a.anchor_hash IS NOT (SELECT h.hash FROM anchor_hashes AS h WHERE h.node_id = a.node_id) \
     ORDER BY a.fact_id";
/// The current facts of status ?1 whose status was set at ?2 or before, in id order.
pub(crate) const FACTS_WITH_STATUS_SINCE: &str = "SELECT fact_id FROM facts \
     WHERE status = ?1 AND superseded_by IS NULL AND status_at <= ?2 ORDER BY fact_id";
/// The id of fact ?1, when one has it.
pub(crate) const FACT_ID_OF: &str = "SELECT fact_id FROM facts WHERE fact_id = ?1";
/// Each node of the JSON list ?1 that is a file, a symbol in one or the repository's root: the
/// file's path (empty for the root) and what the index holds for it, for a symbol its qualified
/// name, and the hash an anchor on the node would keep now, looked up by node, in node order.
pub(crate) const ANCHOR_STATES_OF: &str = "SELECT n.node_id, \
     COALESCE(f.path, sf.path, CASE WHEN n.kind = 'dir' AND n.path = '' THEN '' END), \
     COALESCE(f.content_hash, sf.content_hash), s.qual_name, \
     (SELECT h.hash FROM anchor_hashes AS h WHERE h.node_id = n.node_id) FROM nodes AS n \
     LEFT JOIN files AS f ON n.kind = 'file' AND f.file_id = n.ref_id \
     LEFT JOIN symbols AS s ON n.kind = 'symbol' AND s.symbol_id = n.ref_id \
     LEFT JOIN files AS sf ON sf.file_id = s.file_id \
     WHERE n.node_id IN (SELECT value FROM json_each(?1)) \
     AND COALESCE(f.path, sf.path, CASE WHEN n.kind = 'dir' AND n.path = '' THEN '' END) \
     IS NOT NULL ORDER BY n.node_id";
/// The hash an anchor on each node of the JSON list ?1 would keep now, in node order; a node with
/// none is left out.
pub(crate) const ANCHOR_HASHES_OF: &str = "SELECT node_id, hash FROM anchor_hashes \
     WHERE node_id IN (SELECT value FROM json_each(?1)) AND hash IS NOT NULL ORDER BY node_id";
/// The current facts anchored to any node of the JSON list ?1, each once, in id order.
pub(crate) const FACTS_ANCHORED_TO: &str = "SELECT DISTINCT a.fact_id FROM fact_anchors AS a \
     JOIN facts AS f ON f.fact_id = a.fact_id \
     WHERE a.node_id IN (SELECT value FROM json_each(?1)) AND f.superseded_by IS NULL \
     ORDER BY a.fact_id";
/// The current facts of status ?2 with an anchor on a file of the JSON list ?1 of file nodes, or
/// on a symbol defined in one, each once, in id order.
pub(crate) const FACTS_IN_FILES: &str = "SELECT DISTINCT a.fact_id FROM fact_anchors AS a \
     JOIN facts AS f ON f.fact_id = a.fact_id \
     JOIN nodes AS n ON n.node_id = a.node_id \
     LEFT JOIN symbols AS s ON n.kind = 'symbol' AND s.symbol_id = n.ref_id \
     JOIN nodes AS file ON file.kind = 'file' \
     AND file.ref_id = (CASE n.kind WHEN 'file' THEN n.ref_id ELSE s.file_id END) \
     WHERE file.node_id IN (SELECT value FROM json_each(?1)) AND f.status = ?2 \
     AND f.superseded_by IS NULL ORDER BY a.fact_id";
/// Fact ?1's anchors, in node order.
pub(crate) const FACT_ANCHORS_OF: &str =
    "SELECT node_id, anchor_hash FROM fact_anchors WHERE fact_id = ?1 ORDER BY node_id";

/// How many warnings were given, how many of them are marked, and how many of those resolved: each
/// row one file a warning named.
pub(crate) const WARNING_COUNTS: &str =
    "SELECT count(*), count(resolved), coalesce(sum(resolved), 0) FROM warnings";

/// The warnings of one task, in the order they were recorded, read back as [`crate::warnings`]
/// reads them.
pub(crate) const WARNINGS_OF_TASK: &str = "SELECT warning_id, task_id, ts, kind, src_node, \
    dst_node, strength, resolved FROM warnings WHERE task_id = ?1 ORDER BY warning_id";
/// The warnings not resolved yet (their task not ended) that name, as edited or as warned of, the
/// file at node ?1 or a symbol in it, the latest first.
pub(crate) const OPEN_WARNINGS_ABOUT: &str = "WITH mine AS (SELECT node_id FROM nodes \
    WHERE kind != 'dir' AND path = (SELECT path FROM nodes WHERE node_id = ?1 AND kind = 'file')) \
    SELECT warning_id, task_id, ts, kind, src_node, dst_node, strength, resolved FROM warnings \
    WHERE resolved IS NULL AND (src_node IN (SELECT node_id FROM mine) \
    OR dst_node IN (SELECT node_id FROM mine)) ORDER BY ts DESC, warning_id DESC";
/// The other files the file at node ?1, or a symbol in it, is linked to by an edge of type ?2, ?3
/// or ?4 either way: for each file, type and task an edge was last changed at, the strongest such
/// edge's weight; by type, then strongest first, then by path and task.
pub(crate) const LINKS_OF_FILE: &str = "WITH me AS (SELECT path FROM nodes \
    WHERE node_id = ?1 AND kind = 'file'), \
    mine AS (SELECT node_id FROM nodes WHERE kind != 'dir' AND path = (SELECT path FROM me)), \
    linked AS (SELECT o.path, e.type, e.w, e.last_seq FROM mine AS m JOIN edges AS e \
    ON e.src = m.node_id JOIN nodes AS o ON o.node_id = e.dst \
    WHERE e.type IN (?2, ?3, ?4) AND o.kind != 'dir' \
    UNION ALL SELECT o.path, e.type, e.w, e.last_seq FROM mine AS m JOIN edges AS e \
    ON e.dst = m.node_id JOIN nodes AS o ON o.node_id = e.src \
    WHERE e.type IN (?2, ?3, ?4) AND o.kind != 'dir') \
    SELECT path, type, max(w) AS w, last_seq FROM linked WHERE path != (SELECT path FROM me) \
    GROUP BY path, type, last_seq ORDER BY type, w DESC, path, last_seq";

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

    /// The columns of the tables of what Cortex learns, typed from their table definitions.
    fn expected_learning_columns() -> [(&'static str, Vec<Column>); 7] {
        [
            (
                "interest",
                column_list(&[
                    ("node_id", "INTEGER", 1),
                    ("score", "REAL", 0),
                    ("last_seq", "INTEGER", 0),
                    ("tier", "INTEGER", 0),
                ]),
            ),
            (
                "cochange_stats",
                column_list(&[
                    ("file_a", "INTEGER", 1),
                    ("file_b", "INTEGER", 2),
                    ("count", "INTEGER", 0),
                    ("weighted", "REAL", 0),
                    ("last_commit_seq", "INTEGER", 0),
                ]),
            ),
            (
                "file_commits",
                column_list(&[
                    ("file_id", "INTEGER", 1),
                    ("count", "INTEGER", 0),
                    ("weighted", "REAL", 0),
                ]),
            ),
            (
                "git_state",
                column_list(&[("key", "TEXT", 1), ("value", "TEXT", 0)]),
            ),
            (
                "episodes",
                column_list(&[
                    ("episode_id", "INTEGER", 1),
                    ("task_id", "TEXT", 0),
                    ("localize", "TEXT", 0),
                    ("change", "TEXT", 0),
                    ("verify", "TEXT", 0),
                    ("pitfalls", "TEXT", 0),
                    ("outcome", "TEXT", 0),
                    ("created_at", "INTEGER", 0),
                ]),
            ),
            (
                "archetypes",
                column_list(&[
                    ("archetype_id", "INTEGER", 1),
                    ("seed_terms", "TEXT", 0),
                    ("files", "TEXT", 0),
                    ("episode_count", "INTEGER", 0),
                    ("updated_at", "INTEGER", 0),
                ]),
            ),
            (
                "usage",
                column_list(&[
                    ("session_id", "TEXT", 1),
                    ("turn", "INTEGER", 2),
                    ("ts", "INTEGER", 0),
                    ("model", "TEXT", 0),
                    ("input", "INTEGER", 0),
                    ("cache_write", "INTEGER", 0),
                    ("cache_read", "INTEGER", 0),
                    ("output", "INTEGER", 0),
                    ("reasoning", "INTEGER", 0),
                    ("cache_write_1h", "INTEGER", 0),
                    ("service_tier", "TEXT", 0),
                    ("speed", "TEXT", 0),
                    ("inference_geo", "TEXT", 0),
                    ("response", "BLOB", 0),
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
    fn expected_indexes() -> [(&'static str, Vec<Index>); 19] {
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
                index_list(&[
                    (&["task_id"], true, false),
                    (&["seq"], true, false),
                    (&["session_id", "seq"], false, false),
                ]),
            ),
            (
                "events",
                index_list(&[
                    (&["task_id"], false, false),
                    (&["session_id", "ts"], false, false),
                    (&["task_id"], false, true),
                ]),
            ),
            (
                "cochange_stats",
                index_list(&[
                    (&["file_a", "file_b"], true, false),
                    (&["file_b"], false, false),
                ]),
            ),
            ("git_state", index_list(&[(&["key"], true, false)])),
            ("episodes", index_list(&[(&["task_id"], true, false)])),
            (
                "usage",
                index_list(&[
                    (&["session_id", "turn"], true, false),
                    (&["response"], false, false),
                ]),
            ),
            (
                "injections",
                index_list(&[
                    (&["task_id"], false, false),
                    (&["session_id", "ts"], false, false),
                ]),
            ),
            ("warnings", index_list(&[(&["task_id"], false, false)])),
            (
                "positions",
                index_list(&[
                    (&["session_id", "node_id"], true, false),
                    (&["last_ts"], false, false),
                ]),
            ),
            (
                "notices",
                index_list(&[(&["to_session", "created_at"], false, false)]),
            ),
            ("handoffs", index_list(&[(&["created_at"], false, true)])),
        ]
    }

    /// The indexes of the fact tables: current facts by status (partial), anchors by fact and node
    /// (their key) and by node.
    fn expected_fact_indexes() -> [(&'static str, Vec<Index>); 2] {
        [
            ("facts", index_list(&[(&["status"], false, true)])),
            (
                "fact_anchors",
                index_list(&[
                    (&["fact_id", "node_id"], true, false),
                    (&["node_id"], false, false),
                ]),
            ),
        ]
    }

    /// A review finds the facts whose anchors changed by looking up each anchor's node, never by
    /// scanning every node: it runs after every index, and a repository may hold hundreds of
    /// thousands of nodes. Its plan names no scan and no view built whole.
    #[test]
    fn changed_anchors_are_found_by_lookups() -> Result<(), Box<dyn std::error::Error>> {
        let connection = Connection::open_in_memory()?;
        for migration in MIGRATIONS {
            connection.execute_batch(migration.sql)?;
        }
        let mut statement =
            connection.prepare(&format!("EXPLAIN QUERY PLAN {FACTS_WITH_CHANGED_ANCHORS}"))?;
        let plan: Vec<String> = statement
            .query_map(["candidate", "active"], |row| row.get(3))?
            .collect::<rusqlite::Result<_>>()?;
        assert!(
            plan.iter()
                .all(|step| !step.starts_with("SCAN") && !step.starts_with("MATERIALIZE")),
            "{plan:?}"
        );
        Ok(())
    }

    /// The fact tables, column by column: facts, their anchors, and the facts each task was given.
    fn expected_fact_columns() -> [(&'static str, Vec<Column>); 3] {
        [
            (
                "facts_given",
                column_list(&[("task_id", "TEXT", 1), ("fact_id", "INTEGER", 2)]),
            ),
            (
                "facts",
                column_list(&[
                    ("fact_id", "INTEGER", 1),
                    ("text", "TEXT", 0),
                    ("kind", "TEXT", 0),
                    ("source", "TEXT", 0),
                    ("status", "TEXT", 0),
                    ("status_at", "INTEGER", 0),
                    ("asserted_in", "TEXT", 0),
                    ("uses", "INTEGER", 0),
                    ("created_at", "INTEGER", 0),
                    ("last_verified_at", "INTEGER", 0),
                    ("superseded_by", "INTEGER", 0),
                    ("before_suspect", "TEXT", 0),
                ]),
            ),
            (
                "fact_anchors",
                column_list(&[
                    ("fact_id", "INTEGER", 1),
                    ("node_id", "INTEGER", 2),
                    ("anchor_hash", "BLOB", 0),
                ]),
            ),
        ]
    }

    /// The coordination tables, column by column: positions, notices and handoffs.
    fn expected_coordination_columns() -> [(&'static str, Vec<Column>); 3] {
        [
            (
                "positions",
                column_list(&[
                    ("session_id", "TEXT", 1),
                    ("node_id", "INTEGER", 2),
                    ("mode", "TEXT", 0),
                    ("since", "INTEGER", 0),
                    ("last_ts", "INTEGER", 0),
                ]),
            ),
            (
                "notices",
                column_list(&[
                    ("notice_id", "INTEGER", 1),
                    ("to_session", "TEXT", 0),
                    ("text", "TEXT", 0),
                    ("created_at", "INTEGER", 0),
                    ("delivered_at", "INTEGER", 0),
                    ("about", "TEXT", 0),
                ]),
            ),
            (
                "handoffs",
                column_list(&[
                    ("handoff_id", "INTEGER", 1),
                    ("from_session", "TEXT", 0),
                    ("task_id", "TEXT", 0),
                    ("working_set", "TEXT", 0),
                    ("open_warnings", "TEXT", 0),
                    ("last_failure", "TEXT", 0),
                    ("created_at", "INTEGER", 0),
                    ("consumed_by", "TEXT", 0),
                ]),
            ),
        ]
    }

    /// The injections and warnings tables, column by column.
    fn expected_injection_columns() -> [(&'static str, Vec<Column>); 2] {
        [
            (
                "injections",
                column_list(&[
                    ("injection_id", "INTEGER", 1),
                    ("task_id", "TEXT", 0),
                    ("session_id", "TEXT", 0),
                    ("hook", "TEXT", 0),
                    ("ts", "INTEGER", 0),
                    ("items", "TEXT", 0),
                    ("tokens_est", "INTEGER", 0),
                    ("confidence", "REAL", 0),
                    ("gated_out", "INTEGER", 0),
                    ("holdout", "INTEGER", 0),
                    ("rendered_hash", "BLOB", 0),
                    ("used_count", "INTEGER", 0),
                    ("label", "INTEGER", 0),
                    ("features", "TEXT", 0),
                ]),
            ),
            (
                "warnings",
                column_list(&[
                    ("warning_id", "INTEGER", 1),
                    ("task_id", "TEXT", 0),
                    ("ts", "INTEGER", 0),
                    ("kind", "TEXT", 0),
                    ("src_node", "INTEGER", 0),
                    ("dst_node", "INTEGER", 0),
                    ("strength", "REAL", 0),
                    ("resolved", "INTEGER", 0),
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

    /// The queries a prompt's retrieval makes for each file it lists, and an edit's for the edited
    /// file's tests, start from the rows they ask for, as their plans show, never from every node of
    /// a kind or every co-change pair: on a repository of 10,000 files those walks took most of a
    /// retrieval's time.
    #[test]
    fn a_retrieval_s_queries_start_where_their_index_does() -> Result<(), Box<dyn std::error::Error>>
    {
        let connection = Connection::open_in_memory()?;
        for migration in MIGRATIONS {
            connection.execute_batch(migration.sql)?;
        }
        let plan = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<Vec<String>> {
            let mut statement = connection.prepare(&["EXPLAIN QUERY PLAN ", sql].concat())?;
            statement
                .query_map(params, |row| row.get(3))?
                .collect::<rusqlite::Result<_>>()
        };
        let cases: [(&str, Vec<String>, &str); 5] = [
            (
                "symbols named",
                plan(SYMBOL_NODES_NAMED, &[&"run", &4])?,
                "SEARCH s USING COVERING INDEX symbols_name (name=?)",
            ),
            (
                "a file's symbols",
                plan(SYMBOL_SPANS_IN_FILE, &[&1])?,
                "SEARCH s USING INDEX symbols_file_id (file_id=?)",
            ),
            (
                "the files of nodes",
                plan(FILES_OF_NODES, &[&"[1]"])?,
                "CO-ROUTINE j",
            ),
            (
                "a file's pairs",
                plan(COCHANGE_STATS_OF, &[&"[1]"])?,
                "MERGE (UNION)",
            ),
            (
                "a file's tests",
                plan(TEST_FILES_INTO, &[&1, &0])?,
                "SEARCH e USING INDEX edges_dst_type (dst=? AND type=?)",
            ),
        ];
        for (what, steps, first) in cases {
            assert!(
                steps.first().is_some_and(|step| step.starts_with(first)),
                "{what}: {steps:?}"
            );
            assert!(
                !steps
                    .iter()
                    .any(|step| step.contains("(kind=?)") || step == "SCAN cochange_stats"),
                "{what}: {steps:?}"
            );
        }
        let pairs = plan(COCHANGE_STATS_OF, &[&"[1]"])?;
        assert!(
            pairs
                .iter()
                .any(|step| step.contains("USING INDEX cochange_stats_file_b")),
            "{pairs:?}"
        );
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

    /// A session's edits are found through the index of tasks by session and the index of edits
    /// alone, and its reads through the index of events by session and time, read newest first
    /// without a sort, as the query plans show.
    #[test]
    fn a_sessions_edits_and_reads_are_found_by_their_indexes()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = Connection::open_in_memory()?;
        for migration in MIGRATIONS {
            connection.execute_batch(migration.sql)?;
        }
        let plan = |sql: &str| -> rusqlite::Result<Vec<String>> {
            let mut statement = connection.prepare(&["EXPLAIN QUERY PLAN ", sql].concat())?;
            statement.query_map(("s1", 10), |row| row.get(3))?.collect()
        };
        let edits = plan(EDITS_OF_SESSION)?;
        for index in ["INDEX tasks_session", "INDEX events_edits"] {
            assert!(
                edits.iter().any(|step| step.contains(index)),
                "{index}: {edits:?}"
            );
        }
        let reads = plan(READS_OF_SESSION)?;
        assert!(
            reads
                .iter()
                .any(|step| step.contains("INDEX events_session_ts")),
            "{reads:?}"
        );
        assert!(
            !reads.iter().any(|step| step.contains("TEMP B-TREE")),
            "{reads:?}"
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
    /// imports, indexed by file. The injections table is indexed by task and by session and time,
    /// the warnings table by task.
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
                "archetypes",
                "cochange_stats",
                "edges",
                "episodes",
                "events",
                "fact_anchors",
                "facts",
                "facts_given",
                "file_commits",
                "files",
                "git_state",
                "handoffs",
                "injections",
                "interest",
                "meta",
                "nodes",
                "notices",
                "passes_on",
                "positions",
                "refs",
                "search_index",
                "search_terms",
                "sessions",
                "symbols",
                "tasks",
                "usage",
                "warnings"
            ]
        );
        for (table, expected) in expected_columns()
            .iter()
            .chain(&expected_activity_columns())
            .chain(&expected_learning_columns())
            .chain(&expected_injection_columns())
            .chain(&expected_coordination_columns())
            .chain(&expected_fact_columns())
        {
            assert_eq!(&columns(&connection, table)?, expected, "{table}");
        }
        for (table, expected) in expected_indexes().iter().chain(&expected_fact_indexes()) {
            assert_eq!(&indexes(&connection, table)?, expected, "{table}");
        }
        Ok(())
    }

    /// The foreign keys are the table definitions typed below: each refuses a reference to a
    /// missing row, an edge goes with either of its nodes, and co-change counts and interest go
    /// with their files and nodes.
    #[test]
    fn foreign_keys_match_the_table_definitions() -> Result<(), Box<dyn std::error::Error>> {
        let connection = Connection::open_in_memory()?;
        for migration in MIGRATIONS {
            connection.execute_batch(migration.sql)?;
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
        assert_eq!(
            foreign_keys(&connection, "interest")?,
            [key("nodes", "node_id", "node_id", "CASCADE")]
        );
        assert_eq!(
            foreign_keys(&connection, "cochange_stats")?,
            [
                key("files", "file_a", "file_id", "CASCADE"),
                key("files", "file_b", "file_id", "CASCADE"),
            ]
        );
        assert_eq!(
            foreign_keys(&connection, "file_commits")?,
            [key("files", "file_id", "file_id", "CASCADE")]
        );
        assert_eq!(
            foreign_keys(&connection, "episodes")?,
            [key("tasks", "task_id", "task_id", "NO ACTION")]
        );
        assert_eq!(
            foreign_keys(&connection, "usage")?,
            [key("sessions", "session_id", "session_id", "NO ACTION")]
        );
        // An injection belongs to a stored task and session.
        assert_eq!(
            foreign_keys(&connection, "injections")?,
            [
                key("sessions", "session_id", "session_id", "NO ACTION"),
                key("tasks", "task_id", "task_id", "NO ACTION"),
            ]
        );
        assert_eq!(
            foreign_keys(&connection, "warnings")?,
            [key("tasks", "task_id", "task_id", "NO ACTION")]
        );
        Ok(())
    }
}
