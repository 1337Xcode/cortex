-- The references that go through an import, tied or not, by name: when the definitions of a name
-- come or go, the files holding such a reference are read again, and this finds them without
-- visiting every reference of the name. The condition is written as the query that uses it writes
-- it, which is what lets SQLite use the index.
CREATE INDEX refs_through_imports ON refs (dst_name) WHERE resolution = 'import_path';
