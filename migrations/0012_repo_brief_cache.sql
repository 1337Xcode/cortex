-- Cached repo brief (invalidated on re-index)
CREATE TABLE repo_brief_cache (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    brief_json TEXT NOT NULL,
    computed_at INTEGER NOT NULL,
    index_hash TEXT NOT NULL  -- SHA256 of (files_indexed, node_count, edge_count)
);
