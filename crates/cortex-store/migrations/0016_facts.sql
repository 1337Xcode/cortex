-- Facts: short project rules of at most 240 characters that the user, an agent or a team import
-- wrote, each anchored to the files or symbols it is about with the anchor's hash when it
-- was made, so that a fact whose code changed turns suspect. A corrected fact supersedes the one it
-- corrects, which stays as history, hidden from every reader that asks for current facts.
--
-- Rows hold a rule written in words, its kind, where it came from, its status, node ids and hashes:
-- never code, prompt text or tool output.

CREATE TABLE facts (
    fact_id INTEGER PRIMARY KEY,
    text TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    status TEXT NOT NULL,
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
