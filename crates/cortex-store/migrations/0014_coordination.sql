-- Coordination across the sessions of a repository: where each session is in the map (what it
-- read, edited or claimed, and when), the notices waiting for a session's next hook, and the
-- handoffs a session that ended unfinished leaves for the next one.
--
-- Rows hold session ids, node ids, modes, times, notices that name agents and files (never lines),
-- and JSON of node ids and event numbers: never code, prompt text or a command. One position is
-- kept per session and node: the one that holds longest, which the caller decides.

CREATE TABLE positions (
    session_id TEXT NOT NULL,
    node_id INTEGER NOT NULL,
    mode TEXT NOT NULL,
    since INTEGER NOT NULL,
    last_ts INTEGER NOT NULL,
    PRIMARY KEY (session_id, node_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX positions_last ON positions (last_ts);

CREATE TABLE notices (
    notice_id INTEGER PRIMARY KEY,
    to_session TEXT NOT NULL,
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    delivered_at INTEGER
) STRICT;
CREATE INDEX notices_to ON notices (to_session, created_at);

CREATE TABLE handoffs (
    handoff_id INTEGER PRIMARY KEY,
    from_session TEXT NOT NULL,
    task_id TEXT NOT NULL,
    working_set TEXT NOT NULL,
    open_warnings TEXT NOT NULL,
    last_failure TEXT,
    created_at INTEGER NOT NULL,
    consumed_by TEXT
) STRICT;
CREATE INDEX handoffs_open ON handoffs (created_at) WHERE consumed_by IS NULL;
