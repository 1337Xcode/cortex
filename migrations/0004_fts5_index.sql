CREATE VIRTUAL TABLE nodes_fts USING fts5(
    fqn, kind, file, attributes,
    content='nodes', content_rowid='rowid',
    tokenize='unicode61'
);
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
