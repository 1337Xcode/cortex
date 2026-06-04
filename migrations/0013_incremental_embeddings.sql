-- Migration 0013: Add content_hash to node_embeddings for incremental embedding support.
-- Only re-embed functions whose content has changed (tracked by hash).

ALTER TABLE node_embeddings ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
