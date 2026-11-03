-- Which response each usage row reports, as a hash of the provider's id for the message, so that a
-- response two transcripts report is counted once: a forked session's transcript starts with a
-- copy of the lines of the session it was forked from, each keeping its message id.
--
-- NULL for the rows written before this, which are never taken for copies.

ALTER TABLE usage ADD COLUMN response BLOB;
CREATE INDEX usage_by_response ON usage (response);
