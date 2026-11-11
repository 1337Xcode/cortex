-- Why code looks the way it does: the blame of a span, kept per content hash, and a card for each
-- commit that owns lines. Nothing a person wrote is kept: no subject, no message and no name. A
-- card holds numbers, a kind, object ids and node ids; its terms are the words the caller found
-- worth matching a prompt against.

-- The blame of the lines `first_line` to `last_line` of a file as it stood when its content hashed
-- to `file_hash`. `runs` is a JSON array of `[final_start, final_end, source_start, oid]`, one per
-- run of lines, in file order: one row holds a whole blame, so reading one back is one lookup and
-- replacing or dropping one is one statement. `computed_at` is UTC milliseconds.
CREATE TABLE blame_cache (
    file_hash BLOB NOT NULL,
    first_line INTEGER NOT NULL,
    last_line INTEGER NOT NULL,
    runs TEXT NOT NULL,
    computed_at INTEGER NOT NULL,
    PRIMARY KEY (file_hash, first_line, last_line)
) STRICT, WITHOUT ROWID;

-- Upkeep drops the oldest rows first.
CREATE INDEX blame_cache_computed ON blame_cache (computed_at);

-- A commit as a card. `committed_at` is the committer time in UTC seconds, as git gives it;
-- `carded_at` is UTC milliseconds. `kind` is the fixed name of the kind of change. `pr` is the pull
-- request number the commit came from. `issues` is a JSON array of at most three references as
-- the message wrote them (`#12`, `owner/repo#12`). `has_reason` is 0 or 1. `file_nodes`,
-- `symbol_nodes` and `test_nodes` are JSON arrays of the node ids of the files, symbols and test
-- files the commit changed, each at most 256 long.
CREATE TABLE change_cards (
    oid TEXT NOT NULL PRIMARY KEY,
    first_parent TEXT,
    committed_at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    pr INTEGER,
    issues TEXT NOT NULL,
    has_reason INTEGER NOT NULL,
    file_nodes TEXT NOT NULL,
    symbol_nodes TEXT NOT NULL,
    test_nodes TEXT NOT NULL,
    carded_at INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX change_cards_pr ON change_cards (pr) WHERE pr IS NOT NULL;

-- The words of each card, for matching a prompt's words to past changes by BM25. A card with no
-- word has no row here.
CREATE VIRTUAL TABLE commit_terms USING fts5 (oid UNINDEXED, tokens);
