-- How many search rows hold each word, read from the full-text index itself, so that a lookup can
-- weigh a word by how rare it is. The table is a view over the index and holds nothing of its own.

CREATE VIRTUAL TABLE search_terms USING fts5vocab(search_index, 'row');
