-- What a response's price depends on besides its model and counts: the part of its cache writes
-- made for an hour, which costs more than the five-minute part, and how it was served, its
-- service tier, speed and where it ran.
--
-- Each is NULL where the transcript did not say. The three names are short values the provider
-- reports, such as `standard`, `fast` or `us`, never text from a prompt or an answer.

ALTER TABLE usage ADD COLUMN cache_write_1h INTEGER;
ALTER TABLE usage ADD COLUMN service_tier TEXT;
ALTER TABLE usage ADD COLUMN speed TEXT;
ALTER TABLE usage ADD COLUMN inference_geo TEXT;
