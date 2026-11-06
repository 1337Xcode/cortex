-- The warnings Cortex gave after edits: one row for each file a warning named. `src_node` is what
-- the warning is about (the edited file, or the symbol whose signature changed) and `dst_node` the
-- file it named (the companion, the test, or a file still calling the symbol). Nodes may be
-- deleted with their files later; a warning is history and keeps their numbers. `resolved` is set
-- when the task ends: 1 when the named file was read or edited after the warning, else 0.

CREATE TABLE warnings (
    warning_id INTEGER PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks (task_id),
    ts INTEGER NOT NULL,
    kind TEXT NOT NULL,
    src_node INTEGER NOT NULL,
    dst_node INTEGER NOT NULL,
    strength REAL NOT NULL,
    resolved INTEGER
) STRICT;

CREATE INDEX warnings_task ON warnings (task_id);
