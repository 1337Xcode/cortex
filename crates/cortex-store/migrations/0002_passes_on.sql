-- What each file passes on of what its top-level imports bring in, so that a name can be followed
-- through a file without reading the file again. A row with a name: the file passes the name on,
-- taken under `taken` from the file at `path` (no path: from outside the repository); `position`
-- groups the files one import names. A row without a name: every name of the file at `path`, or,
-- without a path either, of a module outside the repository.
CREATE TABLE passes_on (
    file_id INTEGER NOT NULL REFERENCES files (file_id),
    position INTEGER,
    name TEXT,
    taken TEXT,
    path TEXT
) STRICT;
CREATE INDEX passes_on_file_id ON passes_on (file_id);

-- The files indexed before this table existed have no rows in it: the next index reads every file.
DELETE FROM meta WHERE key = 'cortex_version';
