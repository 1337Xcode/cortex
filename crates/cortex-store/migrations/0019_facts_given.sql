-- The facts each task was given, once each: the task's id and the fact's, so that the recap a
-- session gets after a compaction can carry the rules its task's work orders carried. Ids only.

CREATE TABLE facts_given (
    task_id TEXT NOT NULL,
    fact_id INTEGER NOT NULL,
    PRIMARY KEY (task_id, fact_id)
) STRICT, WITHOUT ROWID;
