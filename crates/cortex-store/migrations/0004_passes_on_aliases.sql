-- The names a file passes a name on as, where they differ from the name its import takes: when
-- the definitions of a name change, an index follows it to those names without reading every row.
-- The condition is written as the query that uses it writes it, which is what lets SQLite use the
-- index.
CREATE INDEX passes_on_aliases ON passes_on (taken) WHERE name <> taken;
