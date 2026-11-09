-- Facts: short project rules of at most 240 characters that the user, an agent or a team import
-- wrote, each anchored to the files or symbols it is about with the anchor's 16-byte hash when it
-- was made, so that a fact whose code changed turns suspect. A corrected fact supersedes the one it
-- corrects, which stays as history, hidden from every reader that asks for current facts. A fact
-- keeps when its status was last set (a suspect one retires after a while) and the task it was
-- first asserted in (asserted again in another task, a candidate turns active).
--
-- Rows hold a rule written in words, its kind, where it came from, its status, a task id, node ids
-- and hashes: never code, prompt text or tool output.

CREATE TABLE facts (
    fact_id INTEGER PRIMARY KEY,
    text TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    status TEXT NOT NULL,
    status_at INTEGER NOT NULL,
    asserted_in TEXT,
    uses INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_verified_at INTEGER NOT NULL,
    superseded_by INTEGER
) STRICT;
CREATE INDEX facts_current ON facts (status) WHERE superseded_by IS NULL;

CREATE TABLE fact_anchors (
    fact_id INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    anchor_hash BLOB NOT NULL,
    PRIMARY KEY (fact_id, node_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX fact_anchors_node ON fact_anchors (node_id);

-- The hash a fact anchored to a node now would keep: the first 16 bytes of a file's content hash,
-- a symbol's body hash. Other nodes, and files whose contents are not read, have none.
CREATE VIEW anchor_hashes AS
SELECT n.node_id AS node_id,
       CASE n.kind
           WHEN 'file' THEN substr(f.content_hash, 1, 16)
           WHEN 'symbol' THEN s.body_hash
       END AS hash
FROM nodes AS n
LEFT JOIN files AS f ON n.kind = 'file' AND f.file_id = n.ref_id
LEFT JOIN symbols AS s ON n.kind = 'symbol' AND s.symbol_id = n.ref_id;
