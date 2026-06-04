-- Add edge_source column and new edge kinds to edges table.
-- Requires recreating the table (SQLite limitation for CHECK constraint changes).
-- Existing edges are migrated with edge_source='ast_direct' and confidence=0.5.

CREATE TABLE edges_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_fqn TEXT NOT NULL REFERENCES nodes(fqn) ON DELETE CASCADE,
    target_fqn TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN (
        'Calls','Imports','Inherits','Implements','HttpLink','DataFlow',
        'Injects','Middleware','Routes','Renders'
    )),
    confidence REAL NOT NULL DEFAULT 0.5 CHECK(confidence >= 0.0 AND confidence <= 1.0),
    edge_source TEXT NOT NULL DEFAULT 'ast_direct'
        CHECK(edge_source IN ('scip', 'framework_adapter', 'ast_direct', 'name_match')),
    attributes TEXT DEFAULT '{}'
);

-- Migrate existing data
INSERT INTO edges_new (id, source_fqn, target_fqn, kind, confidence, edge_source, attributes)
    SELECT id, source_fqn, target_fqn, kind, 0.5, 'ast_direct', attributes FROM edges;

DROP TABLE edges;
ALTER TABLE edges_new RENAME TO edges;

-- Recreate indexes
CREATE INDEX idx_edges_source ON edges(source_fqn);
CREATE INDEX idx_edges_target ON edges(target_fqn);
CREATE INDEX idx_edges_kind ON edges(kind);
CREATE INDEX idx_edges_source_kind ON edges(source_fqn, kind);
CREATE INDEX idx_edges_confidence ON edges(confidence);
CREATE INDEX idx_edges_edge_source ON edges(edge_source);
