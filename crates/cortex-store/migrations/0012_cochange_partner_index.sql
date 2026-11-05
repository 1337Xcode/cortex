-- The pairs a file is the second of, found by an index of their own: a work order asks for every
-- pair of each file it lists, whichever side the file is on, and without this index the second
-- side was a scan of every pair.

CREATE INDEX cochange_stats_file_b ON cochange_stats (file_b);
