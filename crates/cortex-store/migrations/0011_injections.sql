-- What Cortex gave an agent, or would have given: one row for each primer given and for each
-- prompt's retrieval that reached the gate, whatever the gate said.
--
-- `items` lists, as JSON, the files given or weighed: each file's node, the line ranges of its spans
-- and the kind of its reason (`named`, `changes_with`, ...), never a prompt's words, a file's name
-- or code. `features` holds the gate's numbers, as JSON. `rendered_hash` is the hash of the text
-- given, or that would have been. `used_count` and `label` are filled when the task ends. An
-- injection is recorded after its session and, for a prompt's, its task.

CREATE TABLE injections (
    injection_id INTEGER PRIMARY KEY,
    task_id TEXT REFERENCES tasks (task_id),
    session_id TEXT NOT NULL REFERENCES sessions (session_id),
    hook TEXT NOT NULL,
    ts INTEGER NOT NULL,
    items TEXT NOT NULL,
    tokens_est INTEGER NOT NULL,
    confidence REAL,
    gated_out INTEGER NOT NULL,
    holdout INTEGER NOT NULL,
    rendered_hash BLOB,
    used_count INTEGER,
    label INTEGER,
    features TEXT
) STRICT;

CREATE INDEX injections_task ON injections (task_id);
CREATE INDEX injections_session ON injections (session_id, ts);
