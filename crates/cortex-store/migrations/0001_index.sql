-- The index of one repository: facts about the database itself, the files, the symbols defined in
-- them, the names they use, one graph over all of them, and a full-text index of names and paths.
--
-- Text columns hold only paths, names and the fixed names of kinds. Lines are 1-based; byte
-- offsets are 0-based with the end excluded; times are UTC milliseconds except mtime_ns.
--
-- Row ids are never reused (AUTOINCREMENT), so an id kept from an earlier read can only name the
-- row it named then or nothing, never a newer row. Foreign keys refuse a reference to a missing
-- row.

CREATE TABLE meta (
    key TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE files (
    file_id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    lang TEXT,
    bytes INTEGER NOT NULL,
    loc INTEGER,
    content_hash BLOB,
    mtime_ns INTEGER NOT NULL,
    flags INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL
) STRICT;
CREATE INDEX files_lang ON files (lang);

CREATE TABLE symbols (
    symbol_id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL REFERENCES files (file_id),
    parent_id INTEGER,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    qual_name TEXT NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    sig_hash BLOB NOT NULL,
    body_hash BLOB NOT NULL
) STRICT;
CREATE INDEX symbols_file_id ON symbols (file_id);
CREATE INDEX symbols_name ON symbols (name);
CREATE INDEX symbols_qual_name ON symbols (qual_name);

-- A reference whose target is not known yet has no dst_symbol_id, no resolution and confidence 0.
CREATE TABLE refs (
    ref_id INTEGER PRIMARY KEY AUTOINCREMENT,
    src_file_id INTEGER NOT NULL REFERENCES files (file_id),
    src_symbol_id INTEGER,
    dst_name TEXT NOT NULL,
    dst_symbol_id INTEGER REFERENCES symbols (symbol_id),
    kind TEXT NOT NULL,
    line INTEGER NOT NULL,
    resolution TEXT,
    confidence REAL NOT NULL
) STRICT;
CREATE INDEX refs_dst_symbol_id ON refs (dst_symbol_id);
CREATE INDEX refs_src_file_id ON refs (src_file_id);
CREATE INDEX refs_dst_name ON refs (dst_name);

-- One id space for the graph. ref_id is the row id in the table the kind names; a folder has none
-- and is known by its path, which no other folder node shares.
CREATE TABLE nodes (
    node_id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    ref_id INTEGER,
    path TEXT,
    UNIQUE (kind, ref_id)
) STRICT;
CREATE INDEX nodes_path ON nodes (path);
CREATE UNIQUE INDEX nodes_dir_path ON nodes (path) WHERE kind = 'dir';

-- An edge joins two existing nodes and goes when either of them goes.
CREATE TABLE edges (
    src INTEGER NOT NULL REFERENCES nodes (node_id) ON DELETE CASCADE,
    dst INTEGER NOT NULL REFERENCES nodes (node_id) ON DELETE CASCADE,
    type INTEGER NOT NULL,
    w REAL NOT NULL,
    n INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    directed INTEGER NOT NULL,
    PRIMARY KEY (src, dst, type)
) STRICT, WITHOUT ROWID;
CREATE INDEX edges_dst_type ON edges (dst, type);

-- Each row's rowid is its node_id as well, so a row can be replaced without scanning the table.
CREATE VIRTUAL TABLE search_index USING fts5 (node_id UNINDEXED, tokens);
