-- Migration 0005: Vector index for semantic search
-- This migration creates the node_embeddings table for storing vector embeddings.
-- NOTE: sqlite-vec extension is required for actual vector similarity search.
-- When sqlite-vec is not available, semantic search returns a NotEnabled error.

-- Stub: The actual vec0 virtual table requires the sqlite-vec extension.
-- CREATE VIRTUAL TABLE IF NOT EXISTS node_embeddings USING vec0(
--     fqn TEXT PRIMARY KEY,
--     embedding float[768]
-- );

-- For now, create a regular table to store embeddings when available.
CREATE TABLE IF NOT EXISTS node_embeddings (
    fqn TEXT PRIMARY KEY,
    embedding BLOB,
    indexed_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);
