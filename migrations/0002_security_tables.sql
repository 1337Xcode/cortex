CREATE TABLE security_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_fqn TEXT NOT NULL,
    kind TEXT NOT NULL,
    owasp_category TEXT,
    cwe_id TEXT,
    confidence REAL NOT NULL DEFAULT 1.0,
    description TEXT NOT NULL,
    indexed_at INTEGER NOT NULL
);
CREATE INDEX idx_findings_node ON security_findings(node_fqn);
CREATE INDEX idx_findings_kind ON security_findings(kind);

CREATE TABLE taint_paths (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_fqn TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    sink_fqn TEXT NOT NULL,
    sink_kind TEXT NOT NULL,
    path_json TEXT NOT NULL,
    confidence REAL NOT NULL,
    cwe_id TEXT,
    indexed_at INTEGER NOT NULL
);
CREATE INDEX idx_taint_source ON taint_paths(source_fqn);
CREATE INDEX idx_taint_sink ON taint_paths(sink_fqn);

CREATE TABLE sbom_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    version TEXT,
    license TEXT,
    source_file TEXT NOT NULL,
    import_fqn TEXT NOT NULL,
    indexed_at INTEGER NOT NULL
);
