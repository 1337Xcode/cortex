-- What a suspect fact was before its code changed: a candidate or an active fact. Said again, a
-- suspect candidate turns active only as a candidate would (the user's word, or an agent's in a
-- task other than the first one to state it), so an agent cannot confirm its own rule by editing
-- the code it is about. Rows made suspect before this hold none and are taken as candidates.

ALTER TABLE facts ADD COLUMN before_suspect TEXT;
