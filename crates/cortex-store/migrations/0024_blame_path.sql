-- The blame cache keyed by the file's path as well as its content: two files with the same content
-- (a package's `__init__.py`, a copied module) can have been written by different commits, and a
-- blame says who wrote one file's lines. The table is a cache, so its rows are dropped and blamed
-- again when asked for.

DROP TABLE blame_cache;

-- The blame of the lines `first_line` to `last_line` of the file at `path`, as it stood when its
-- content hashed to `file_hash`. `path` is the repository-relative path, `/` between its names.
-- `runs` is a JSON array of `[final_start, final_end, source_start, oid]`, one per run of lines,
-- in file order. `computed_at` is UTC milliseconds.
CREATE TABLE blame_cache (
    path TEXT NOT NULL,
    file_hash BLOB NOT NULL,
    first_line INTEGER NOT NULL,
    last_line INTEGER NOT NULL,
    runs TEXT NOT NULL,
    computed_at INTEGER NOT NULL,
    PRIMARY KEY (path, file_hash, first_line, last_line)
) STRICT, WITHOUT ROWID;

-- Upkeep drops the oldest rows first.
CREATE INDEX blame_cache_computed ON blame_cache (computed_at);
