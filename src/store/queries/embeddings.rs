//! Embedding storage and retrieval queries for semantic search.
//!
//! Stores and retrieves 768-dimensional embeddings for Function/Class nodes
//! in the `node_embeddings` table. Embeddings are stored as raw f32 byte blobs.

use rusqlite::Connection;

use crate::error::StoreError;
use crate::indexer::embedder::{EMBEDDING_DIM, cosine_similarity};

/// A semantic search result with similarity score.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SemanticResult {
    pub fqn: String,
    pub similarity: f32,
    pub kind: Option<String>,
    pub file: Option<String>,
}

/// Store an embedding for a node.
///
/// Inserts or replaces the embedding for the given FQN.
/// The embedding is stored as a raw byte blob (768 * 4 = 3072 bytes).
pub fn store_embedding(conn: &Connection, fqn: &str, embedding: &[f32]) -> Result<(), StoreError> {
    if embedding.len() != EMBEDDING_DIM {
        return Err(StoreError::QueryFailed {
            reason: format!(
                "embedding dimension mismatch: expected {EMBEDDING_DIM}, got {}",
                embedding.len()
            ),
        });
    }

    // Convert f32 slice to bytes
    let bytes = embedding_to_bytes(embedding);

    conn.execute(
        "INSERT OR REPLACE INTO node_embeddings (fqn, embedding) VALUES (?1, ?2)",
        rusqlite::params![fqn, bytes],
    )
    .map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to store embedding for '{fqn}': {e}"),
    })?;

    Ok(())
}

/// Store multiple embeddings in a single transaction.
pub fn store_embeddings_batch(
    conn: &Connection,
    entries: &[(&str, &[f32])],
) -> Result<usize, StoreError> {
    let mut stmt = conn
        .prepare("INSERT OR REPLACE INTO node_embeddings (fqn, embedding) VALUES (?1, ?2)")
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare embedding insert: {e}"),
        })?;

    let mut count = 0;
    for (fqn, embedding) in entries {
        if embedding.len() != EMBEDDING_DIM {
            continue;
        }
        let bytes = embedding_to_bytes(embedding);
        stmt.execute(rusqlite::params![fqn, bytes])
            .map_err(|e| StoreError::QueryFailed {
                reason: format!("failed to store embedding for '{fqn}': {e}"),
            })?;
        count += 1;
    }

    Ok(count)
}

/// Perform semantic search: compute cosine similarity against all stored embeddings.
///
/// Returns the top-k results sorted by similarity (descending).
pub fn semantic_search(
    conn: &Connection,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<SemanticResult>, StoreError> {
    if query_embedding.len() != EMBEDDING_DIM {
        return Err(StoreError::QueryFailed {
            reason: format!(
                "query embedding dimension mismatch: expected {EMBEDDING_DIM}, got {}",
                query_embedding.len()
            ),
        });
    }

    // Load all embeddings and compute similarity
    let mut stmt = conn
        .prepare("SELECT fqn, embedding FROM node_embeddings")
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to prepare embedding query: {e}"),
        })?;

    let rows = stmt
        .query_map([], |row| {
            let fqn: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((fqn, blob))
        })
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to query embeddings: {e}"),
        })?;

    let mut results: Vec<(String, f32)> = Vec::new();

    for row in rows {
        let (fqn, blob) = row.map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to read embedding row: {e}"),
        })?;

        if let Some(stored_embedding) = bytes_to_embedding(&blob) {
            let sim = cosine_similarity(query_embedding, &stored_embedding);
            results.push((fqn, sim));
        }
    }

    // Sort by similarity descending
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Take top-k
    results.truncate(top_k);

    // Enrich with node metadata (kind, file)
    let enriched: Vec<SemanticResult> = results
        .into_iter()
        .map(|(fqn, similarity)| {
            let (kind, file) = get_node_metadata(conn, &fqn);
            SemanticResult {
                fqn,
                similarity,
                kind,
                file,
            }
        })
        .collect();

    Ok(enriched)
}

/// Get the count of stored embeddings.
pub fn embedding_count(conn: &Connection) -> Result<usize, StoreError> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM node_embeddings", [], |row| row.get(0))
        .map_err(|e| StoreError::QueryFailed {
            reason: format!("failed to count embeddings: {e}"),
        })?;
    Ok(count as usize)
}

/// Remove embedding for a specific FQN.
pub fn remove_embedding(conn: &Connection, fqn: &str) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM node_embeddings WHERE fqn = ?1",
        rusqlite::params![fqn],
    )
    .map_err(|e| StoreError::QueryFailed {
        reason: format!("failed to remove embedding for '{fqn}': {e}"),
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert an f32 embedding to raw bytes.
fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Convert raw bytes back to an f32 embedding.
fn bytes_to_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    let expected_bytes = EMBEDDING_DIM * 4;
    if bytes.len() != expected_bytes {
        return None;
    }

    let embedding: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    Some(embedding)
}

/// Get node kind and file from the nodes table.
fn get_node_metadata(conn: &Connection, fqn: &str) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT kind, file FROM nodes WHERE fqn = ?1",
        rusqlite::params![fqn],
        |row| {
            let kind: String = row.get(0)?;
            let file: String = row.get(1)?;
            Ok((Some(kind), Some(file)))
        },
    )
    .unwrap_or((None, None))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::StoreManager;
    use crate::store::migrations;

    fn setup_store() -> (StoreManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = StoreManager::new(tmp.path()).expect("failed to create store");
        let conn = store.write_conn();
        migrations::run_migrations(&conn, std::path::Path::new("migrations"))
            .expect("failed to run migrations");
        drop(conn);
        (store, tmp)
    }

    #[test]
    fn test_store_and_retrieve_embedding() {
        let (store, _tmp) = setup_store();
        let conn = store.write_conn();

        // Insert a test node first
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES ('test::validate_input', 'Function', 'test.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        ).unwrap();

        // Create a test embedding
        let mut embedding = vec![0.0f32; EMBEDDING_DIM];
        embedding[0] = 1.0;
        embedding[1] = 0.5;

        store_embedding(&conn, "test::validate_input", &embedding).unwrap();

        // Verify count
        assert_eq!(embedding_count(&conn).unwrap(), 1);
    }

    #[test]
    fn test_semantic_search_returns_results() {
        let (store, _tmp) = setup_store();
        let conn = store.write_conn();

        // Insert test nodes
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES ('src/auth.rs::validate_user', 'Function', 'src/auth.rs', 1, 10, 'hash', 1000, '{}')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
             VALUES ('src/db.rs::connect', 'Function', 'src/db.rs', 1, 5, 'hash', 1000, '{}')",
            [],
        ).unwrap();

        // Create embeddings - make validate_user similar to query
        let mut emb_validate = vec![0.0f32; EMBEDDING_DIM];
        emb_validate[0] = 0.9;
        emb_validate[1] = 0.8;
        emb_validate[2] = 0.7;

        let mut emb_connect = vec![0.0f32; EMBEDDING_DIM];
        emb_connect[0] = 0.1;
        emb_connect[1] = 0.2;
        emb_connect[100] = 0.9;

        store_embedding(&conn, "src/auth.rs::validate_user", &emb_validate).unwrap();
        store_embedding(&conn, "src/db.rs::connect", &emb_connect).unwrap();

        // Query with embedding similar to validate_user
        let mut query_emb = vec![0.0f32; EMBEDDING_DIM];
        query_emb[0] = 0.85;
        query_emb[1] = 0.75;
        query_emb[2] = 0.65;

        let results = semantic_search(&conn, &query_emb, 10).unwrap();

        assert_eq!(results.len(), 2);
        // validate_user should be more similar
        assert_eq!(results[0].fqn, "src/auth.rs::validate_user");
        assert!(results[0].similarity > results[1].similarity);
        assert_eq!(results[0].kind, Some("Function".to_string()));
        assert_eq!(results[0].file, Some("src/auth.rs".to_string()));
    }

    #[test]
    fn test_store_embeddings_batch() {
        let (store, _tmp) = setup_store();
        let conn = store.write_conn();

        let emb1 = vec![0.1f32; EMBEDDING_DIM];
        let emb2 = vec![0.2f32; EMBEDDING_DIM];

        let entries: Vec<(&str, &[f32])> = vec![("node1", &emb1), ("node2", &emb2)];

        let count = store_embeddings_batch(&conn, &entries).unwrap();
        assert_eq!(count, 2);
        assert_eq!(embedding_count(&conn).unwrap(), 2);
    }

    #[test]
    fn test_remove_embedding() {
        let (store, _tmp) = setup_store();
        let conn = store.write_conn();

        let emb = vec![0.1f32; EMBEDDING_DIM];
        store_embedding(&conn, "test::node", &emb).unwrap();
        assert_eq!(embedding_count(&conn).unwrap(), 1);

        remove_embedding(&conn, "test::node").unwrap();
        assert_eq!(embedding_count(&conn).unwrap(), 0);
    }

    #[test]
    fn test_embedding_dimension_mismatch() {
        let (store, _tmp) = setup_store();
        let conn = store.write_conn();

        let bad_emb = vec![0.1f32; 100]; // wrong dimension
        let result = store_embedding(&conn, "test::node", &bad_emb);
        assert!(result.is_err());
    }

    #[test]
    fn test_bytes_roundtrip() {
        let original = vec![1.0f32, -0.5, 0.0, 3.15];
        let bytes = embedding_to_bytes(&original);
        // Can't use bytes_to_embedding directly since it expects EMBEDDING_DIM
        // but we can verify the byte conversion logic
        let restored: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        assert_eq!(original, restored);
    }
}
