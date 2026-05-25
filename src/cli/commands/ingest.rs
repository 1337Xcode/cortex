//! Ingest command: add documents to the knowledge graph.
//!
//! `cortex ingest ./docs` ingests markdown, text, CSV, and PDF files
//! into the graph as document nodes. No LLM calls, no cloud.
//! Text is extracted locally and stored as searchable nodes.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::Digest;

use crate::store::db::StoreManager;

/// Supported file extensions for ingestion.
const SUPPORTED_EXTENSIONS: &[&str] = &["md", "txt", "csv", "rst", "html", "yaml", "json", "pdf"];

/// Run the ingest command: walk the given path and ingest documents.
pub fn run(store: &StoreManager, path: &Path, types: &str) -> Result<(), anyhow::Error> {
    let allowed_types: Vec<&str> = types.split(',').map(|s| s.trim()).collect();

    if !path.exists() {
        anyhow::bail!("path does not exist: {}", path.display());
    }

    let mut ingested = 0usize;
    let mut skipped = 0usize;
    let mut errors = 0usize;

    if path.is_file() {
        match ingest_file(store, path, &allowed_types) {
            Ok(true) => ingested += 1,
            Ok(false) => skipped += 1,
            Err(e) => {
                eprintln!("warning: failed to ingest '{}': {}", path.display(), e);
                errors += 1;
            }
        }
    } else {
        // Walk directory
        for entry in walkdir::WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }

            match ingest_file(store, entry.path(), &allowed_types) {
                Ok(true) => ingested += 1,
                Ok(false) => skipped += 1,
                Err(e) => {
                    eprintln!(
                        "warning: failed to ingest '{}': {}",
                        entry.path().display(),
                        e
                    );
                    errors += 1;
                }
            }
        }
    }

    println!(
        "Ingest complete: {} documents ingested, {} skipped, {} errors",
        ingested, skipped, errors
    );

    Ok(())
}

/// Ingest a single file. Returns Ok(true) if ingested, Ok(false) if skipped.
fn ingest_file(
    store: &StoreManager,
    file_path: &Path,
    allowed_types: &[&str],
) -> Result<bool, anyhow::Error> {
    let extension = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Check if extension is supported and allowed
    if !SUPPORTED_EXTENSIONS.contains(&extension) {
        return Ok(false);
    }
    if !allowed_types.contains(&extension) {
        return Ok(false);
    }

    // Extract text content based on file type
    let (content, doc_type) = extract_content(file_path, extension)?;

    // Generate a FQN for the document node
    let fqn = format!("doc::{}", file_path.display());

    // Compute file hash for deduplication
    let file_bytes = std::fs::read(file_path)?;
    let hash = format!("{:x}", sha2::Sha256::digest(&file_bytes));

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Build attributes JSON
    let attributes = serde_json::json!({
        "doc_type": doc_type,
        "file_size": file_bytes.len(),
        "content_preview": truncate_content(&content, 500),
    });

    // Insert or update the document node
    let conn = store.write_conn();
    conn.execute(
        "INSERT OR REPLACE INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes)
         VALUES (?1, 'Document', ?2, 0, 0, ?3, ?4, ?5)",
        rusqlite::params![
            fqn,
            file_path.to_string_lossy().as_ref(),
            hash,
            now as i64,
            attributes.to_string(),
        ],
    )?;

    // Try to link document to code nodes by finding mentioned symbols
    link_document_to_code(&conn, &fqn, &content)?;

    Ok(true)
}

/// Extract text content from a file based on its extension.
fn extract_content(
    file_path: &Path,
    extension: &str,
) -> Result<(String, &'static str), anyhow::Error> {
    match extension {
        "md" | "txt" | "rst" | "html" => {
            let content = std::fs::read_to_string(file_path)?;
            let doc_type = match extension {
                "md" => "markdown",
                "txt" => "plaintext",
                "rst" => "restructuredtext",
                "html" => "html",
                _ => "text",
            };
            Ok((content, doc_type))
        }
        "csv" => {
            let content = std::fs::read_to_string(file_path)?;
            // Parse headers as a summary
            let first_line = content.lines().next().unwrap_or("");
            let summary = format!(
                "CSV headers: {}\nRows: {}",
                first_line,
                content.lines().count().saturating_sub(1)
            );
            Ok((summary, "csv"))
        }
        "yaml" | "json" => {
            let content = std::fs::read_to_string(file_path)?;
            Ok((content, if extension == "yaml" { "yaml" } else { "json" }))
        }
        "pdf" => {
            // PDF text extraction requires a C library (poppler) which breaks zero-dep.
            // Store as a reference node with filename and file size.
            let metadata = std::fs::metadata(file_path)?;
            let content = format!(
                "PDF reference: {} ({} bytes)",
                file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown"),
                metadata.len()
            );
            Ok((content, "pdf_reference"))
        }
        _ => Ok((String::new(), "unknown")),
    }
}

/// Truncate content to a maximum length, appending "..." if truncated.
fn truncate_content(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        content.to_string()
    } else {
        format!("{}...", &content[..max_len])
    }
}

/// Link a document node to code nodes when the document mentions known function/class names.
fn link_document_to_code(
    conn: &rusqlite::Connection,
    doc_fqn: &str,
    content: &str,
) -> Result<(), anyhow::Error> {
    // Get all known symbol names (just the short name, not the full FQN)
    let mut stmt = conn.prepare(
        "SELECT fqn FROM nodes WHERE kind IN ('Function', 'Class', 'Method') LIMIT 1000",
    )?;

    let fqns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    let content_lower = content.to_lowercase();

    for fqn in &fqns {
        // Extract the short name from the FQN (after the last ::)
        let short_name = fqn.rsplit("::").next().unwrap_or(fqn);
        if short_name.len() < 3 {
            continue;
        }

        // Check if the document mentions this symbol
        if content_lower.contains(&short_name.to_lowercase()) {
            conn.execute(
                "INSERT OR IGNORE INTO edges (source_fqn, target_fqn, kind, confidence, attributes)
                 VALUES (?1, ?2, 'References', 0.5, '{}')",
                rusqlite::params![doc_fqn, fqn],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_content_short() {
        let result = truncate_content("hello", 10);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_content_long() {
        let result = truncate_content("hello world this is a long string", 10);
        assert_eq!(result, "hello worl...");
    }

    #[test]
    fn test_extract_content_txt() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "hello world").unwrap();
        let (content, doc_type) = extract_content(tmp.path(), "txt").unwrap();
        assert_eq!(content, "hello world");
        assert_eq!(doc_type, "plaintext");
    }

    #[test]
    fn test_extract_content_csv() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "name,age\nAlice,30\nBob,25").unwrap();
        let (content, doc_type) = extract_content(tmp.path(), "csv").unwrap();
        assert!(content.contains("CSV headers: name,age"));
        assert!(content.contains("Rows: 2"));
        assert_eq!(doc_type, "csv");
    }

    #[test]
    fn test_extract_content_pdf() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "fake pdf content").unwrap();
        let (content, doc_type) = extract_content(tmp.path(), "pdf").unwrap();
        assert!(content.contains("PDF reference"));
        assert_eq!(doc_type, "pdf_reference");
    }
}
