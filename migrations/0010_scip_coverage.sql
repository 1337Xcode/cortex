-- Track SCIP coverage per file
CREATE TABLE scip_coverage (
    file TEXT PRIMARY KEY,
    has_scip_data INTEGER NOT NULL DEFAULT 0,
    symbols_resolved INTEGER NOT NULL DEFAULT 0,
    indexed_at INTEGER NOT NULL
);

-- Index health metrics (singleton row, updated on each index run)
CREATE TABLE index_health (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    files_indexed INTEGER NOT NULL DEFAULT 0,
    node_count INTEGER NOT NULL DEFAULT 0,
    edge_count INTEGER NOT NULL DEFAULT 0,
    scip_coverage_percent REAL NOT NULL DEFAULT 0.0,
    last_index_at INTEGER NOT NULL DEFAULT 0,
    frameworks_detected TEXT NOT NULL DEFAULT '[]',
    health_status TEXT NOT NULL DEFAULT 'unknown'
);

INSERT INTO index_health (id, files_indexed, node_count, edge_count, last_index_at)
    VALUES (1, 0, 0, 0, 0);
