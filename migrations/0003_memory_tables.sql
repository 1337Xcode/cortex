CREATE TABLE observations (
    id TEXT PRIMARY KEY,
    node_fqn TEXT NOT NULL,
    observation_text TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    node_hash_at_write TEXT NOT NULL,
    written_at INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','stale','archived')),
    stale_reason TEXT
);
CREATE INDEX idx_obs_node ON observations(node_fqn);
CREATE INDEX idx_obs_status ON observations(status);

CREATE TABLE architectural_decisions (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'proposed' CHECK(status IN ('proposed','accepted','deprecated')),
    linked_fqn TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE change_notes (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE bundle_metadata (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    format_version INTEGER NOT NULL,
    last_export_at INTEGER,
    export_checksum TEXT,
    repo_root_hash TEXT
);
INSERT INTO bundle_metadata(id, format_version) VALUES(1, 1);
