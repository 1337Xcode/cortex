CREATE TABLE nodes (
    fqn TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('Function','Class','Module','Route','Interface','Type')),
    file TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    file_hash TEXT NOT NULL,
    indexed_at INTEGER NOT NULL,
    attributes TEXT DEFAULT '{}'
);
CREATE INDEX idx_nodes_file ON nodes(file);
CREATE INDEX idx_nodes_kind ON nodes(kind);

CREATE TABLE edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_fqn TEXT NOT NULL REFERENCES nodes(fqn) ON DELETE CASCADE,
    target_fqn TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('Calls','Imports','Inherits','Implements','HttpLink','DataFlow')),
    confidence REAL NOT NULL DEFAULT 1.0 CHECK(confidence >= 0.0 AND confidence <= 1.0),
    attributes TEXT DEFAULT '{}'
);
CREATE INDEX idx_edges_source ON edges(source_fqn);
CREATE INDEX idx_edges_target ON edges(target_fqn);
CREATE INDEX idx_edges_kind ON edges(kind);
CREATE INDEX idx_edges_source_kind ON edges(source_fqn, kind);

CREATE TABLE file_snapshots (
    file TEXT PRIMARY KEY,
    file_hash TEXT NOT NULL,
    node_count INTEGER NOT NULL DEFAULT 0,
    indexed_at INTEGER NOT NULL
);
