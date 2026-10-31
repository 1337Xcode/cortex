-- What agents did in the repository: their sessions, the task each prompt starts, and every event
-- their hooks or the file watcher reported.
--
-- Rows hold ids, fixed names of kinds, paths, line ranges, times, sizes and exit codes. A prompt
-- is kept as its hash; its first 200 characters only when the user asks. Commands, search
-- patterns and tool output are never kept. `events.meta` is JSON of such facts only. Times are
-- UTC milliseconds; lines are 1-based. Event numbers are never reused (AUTOINCREMENT), so they
-- order events even after old ones are deleted.

CREATE TABLE sessions (
    session_id TEXT NOT NULL PRIMARY KEY,
    agent TEXT NOT NULL,
    agent_version TEXT,
    model TEXT,
    parent_session_id TEXT,
    cwd TEXT,
    transcript_path TEXT,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    status TEXT NOT NULL
) STRICT;

CREATE TABLE tasks (
    task_id TEXT NOT NULL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions (session_id),
    seq INTEGER NOT NULL UNIQUE,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    prompt_hash BLOB,
    prompt_excerpt TEXT,
    holdout INTEGER,
    outcome TEXT,
    outcome_evidence TEXT
) STRICT;

CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT,
    session_id TEXT,
    ts INTEGER NOT NULL,
    kind TEXT NOT NULL,
    node_id INTEGER,
    path TEXT,
    range_start INTEGER,
    range_end INTEGER,
    meta TEXT NOT NULL
) STRICT;
CREATE INDEX events_task ON events (task_id);
CREATE INDEX events_session_ts ON events (session_id, ts);
