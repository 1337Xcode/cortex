-- What each coordination notice is about, apart from how it is worded: the agent as it is named,
-- the file, and what the agent did there, as one key. A session is sent or given one happening
-- once, whichever side told it and in whatever words. Rows made before this hold none.

ALTER TABLE notices ADD COLUMN about TEXT;
