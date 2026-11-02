-- What Cortex learns: each node's interest, the co-change history git gives, each task's
-- episode, the archetypes episodes cluster into, and the token usage agents report.
--
-- Rows hold ids, fixed names, counts, weights, times and token counts. Episodes and archetypes
-- hold JSON of node ids, runner names, exit codes and counts, never prompt text, commands or
-- code. Times are UTC milliseconds. Weights decay lazily: a row keeps its value as of the task
-- (`last_seq`) or commit (`last_commit_seq`) it was last updated at.

CREATE TABLE interest (
    node_id INTEGER NOT NULL PRIMARY KEY REFERENCES nodes (node_id) ON DELETE CASCADE,
    score REAL NOT NULL,
    last_seq INTEGER NOT NULL,
    tier INTEGER NOT NULL
) STRICT;

CREATE TABLE cochange_stats (
    file_a INTEGER NOT NULL REFERENCES files (file_id) ON DELETE CASCADE,
    file_b INTEGER NOT NULL REFERENCES files (file_id) ON DELETE CASCADE,
    count INTEGER NOT NULL,
    weighted REAL NOT NULL,
    last_commit_seq INTEGER NOT NULL,
    PRIMARY KEY (file_a, file_b)
) STRICT, WITHOUT ROWID;

CREATE TABLE file_commits (
    file_id INTEGER NOT NULL PRIMARY KEY REFERENCES files (file_id) ON DELETE CASCADE,
    count INTEGER NOT NULL,
    weighted REAL NOT NULL
) STRICT;

CREATE TABLE git_state (
    key TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE episodes (
    episode_id INTEGER PRIMARY KEY,
    task_id TEXT NOT NULL UNIQUE REFERENCES tasks (task_id),
    localize TEXT NOT NULL,
    change TEXT NOT NULL,
    verify TEXT NOT NULL,
    pitfalls TEXT NOT NULL,
    outcome TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE archetypes (
    archetype_id INTEGER PRIMARY KEY,
    seed_terms TEXT NOT NULL,
    files TEXT NOT NULL,
    episode_count INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE usage (
    session_id TEXT NOT NULL REFERENCES sessions (session_id),
    turn INTEGER NOT NULL,
    ts INTEGER NOT NULL,
    model TEXT,
    input INTEGER NOT NULL,
    cache_write INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    output INTEGER NOT NULL,
    reasoning INTEGER,
    PRIMARY KEY (session_id, turn)
) STRICT;
