-- The repository's root as an anchor: a project fact that holds for the whole repository (a
-- global convention) is anchored to the root's folder node. The root never changes as code does,
-- so the hash an anchor on it keeps is a constant, 16 zero bytes. Every other folder, a file whose
-- contents are not read and a node that is gone still have none.

DROP VIEW anchor_hashes;
CREATE VIEW anchor_hashes AS
SELECT n.node_id AS node_id,
       CASE n.kind
           WHEN 'file' THEN substr(f.content_hash, 1, 16)
           WHEN 'symbol' THEN s.body_hash
           WHEN 'dir' THEN CASE WHEN n.path = '' THEN zeroblob(16) END
       END AS hash
FROM nodes AS n
LEFT JOIN files AS f ON n.kind = 'file' AND f.file_id = n.ref_id
LEFT JOIN symbols AS s ON n.kind = 'symbol' AND s.symbol_id = n.ref_id;
