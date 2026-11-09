-- What tasks read next: from a file, the file tasks went on to read (or change) right after it,
-- before their first change. `weight` is what the tasks added, as the learner weighs their outcomes,
-- faded with the tasks since `last_seq`; `count` is how many tasks went that way. Node ids only. A
-- transition goes with either of its files' nodes.

CREATE TABLE transitions (
    src INTEGER NOT NULL REFERENCES nodes (node_id) ON DELETE CASCADE,
    dst INTEGER NOT NULL REFERENCES nodes (node_id) ON DELETE CASCADE,
    weight REAL NOT NULL,
    count INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    PRIMARY KEY (src, dst)
) STRICT, WITHOUT ROWID;

CREATE INDEX transitions_dst ON transitions (dst);
