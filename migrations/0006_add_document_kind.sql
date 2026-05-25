-- Add 'Document' to the allowed node kinds.
-- SQLite does not support ALTER TABLE to modify CHECK constraints,
-- so we recreate the table with the expanded constraint.

CREATE TABLE nodes_new (
    fqn TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('Function','Class','Module','Route','Interface','Type','Enum','Constant','TypeAlias','Trait','Namespace','Document')),
    file TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    file_hash TEXT NOT NULL,
    indexed_at INTEGER NOT NULL,
    attributes TEXT DEFAULT '{}'
);

INSERT INTO nodes_new SELECT * FROM nodes;
DROP TABLE nodes;
ALTER TABLE nodes_new RENAME TO nodes;

CREATE INDEX idx_nodes_file ON nodes(file);
CREATE INDEX idx_nodes_kind ON nodes(kind);

-- Recreate FTS5 triggers (DROP TABLE nodes cascaded them)
DROP TRIGGER IF EXISTS nodes_ai;
DROP TRIGGER IF EXISTS nodes_ad;
DROP TRIGGER IF EXISTS nodes_au;

CREATE TRIGGER nodes_ai AFTER INSERT ON nodes BEGIN
    INSERT INTO nodes_fts(rowid, fqn, kind, file, attributes)
    VALUES (new.rowid, new.fqn, new.kind, new.file, new.attributes);
END;
CREATE TRIGGER nodes_ad AFTER DELETE ON nodes BEGIN
    INSERT INTO nodes_fts(nodes_fts, rowid, fqn, kind, file, attributes)
    VALUES('delete', old.rowid, old.fqn, old.kind, old.file, old.attributes);
END;
CREATE TRIGGER nodes_au AFTER UPDATE ON nodes BEGIN
    INSERT INTO nodes_fts(nodes_fts, rowid, fqn, kind, file, attributes)
    VALUES('delete', old.rowid, old.fqn, old.kind, old.file, old.attributes);
    INSERT INTO nodes_fts(rowid, fqn, kind, file, attributes)
    VALUES (new.rowid, new.fqn, new.kind, new.file, new.attributes);
END;
