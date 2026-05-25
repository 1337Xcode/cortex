//! Schema migration runner for the Cortex graph store.
//!
//! Applies numbered SQL migration files idempotently on startup. Each migration
//! runs in its own transaction. The `schema_versions` table tracks which
//! migrations have been applied.

use std::fs;
use std::path::Path;

use rusqlite::Connection;

use crate::error::MigrationError;

/// Ensures the `schema_versions` tracking table exists.
///
/// This table records which migrations have been applied so that subsequent
/// runs skip already-applied files.
fn ensure_schema_versions_table(conn: &Connection) -> Result<(), MigrationError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_versions (
            version  INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL,
            filename TEXT NOT NULL
        );",
    )
    .map_err(|e| MigrationError::SqlExecutionFailed {
        file: "<schema_versions setup>".to_string(),
        reason: e.to_string(),
    })
}

/// Extracts the numeric version prefix from a migration filename.
///
/// Expects filenames like `0001_initial_schema.sql`. Returns `None` if the
/// filename does not start with a parseable integer prefix before the first `_`.
fn parse_version(filename: &str) -> Option<i64> {
    let prefix = filename.split('_').next()?;
    prefix.parse::<i64>().ok()
}

/// Runs all pending migrations from `migrations_dir` against `conn`.
///
/// Returns the list of newly applied migration filenames on success.
///
/// # Errors
///
/// Returns `MigrationError::FileReadFailed` if a migration file cannot be read,
/// or `MigrationError::SqlExecutionFailed` if a migration's SQL fails to execute.
pub fn run_migrations(
    conn: &Connection,
    migrations_dir: &Path,
) -> Result<Vec<String>, MigrationError> {
    ensure_schema_versions_table(conn)?;

    // Collect all .sql files from the migrations directory.
    let entries = fs::read_dir(migrations_dir).map_err(|e| MigrationError::FileReadFailed {
        file: migrations_dir.display().to_string(),
        reason: e.to_string(),
    })?;

    let mut migration_files: Vec<(i64, String)> = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| MigrationError::FileReadFailed {
            file: migrations_dir.display().to_string(),
            reason: e.to_string(),
        })?;

        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.ends_with(".sql") {
            continue;
        }

        if let Some(version) = parse_version(&filename) {
            migration_files.push((version, filename));
        }
    }

    // Sort by version number (numeric order).
    migration_files.sort_by_key(|(version, _)| *version);

    // Determine which migrations have already been applied.
    let applied: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT version FROM schema_versions ORDER BY version")
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: "<schema_versions query>".to_string(),
                reason: e.to_string(),
            })?;

        stmt.query_map([], |row| row.get(0))
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: "<schema_versions query>".to_string(),
                reason: e.to_string(),
            })?
            .filter_map(|r| r.ok())
            .collect()
    };

    let mut newly_applied: Vec<String> = Vec::new();

    for (version, filename) in &migration_files {
        if applied.contains(version) {
            continue;
        }

        // Read the SQL content from disk.
        let file_path = migrations_dir.join(filename);
        let sql = fs::read_to_string(&file_path).map_err(|e| MigrationError::FileReadFailed {
            file: filename.clone(),
            reason: e.to_string(),
        })?;

        // Execute the migration in its own transaction.
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: filename.clone(),
                reason: e.to_string(),
            })?;

        tx.execute_batch(&sql)
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: filename.clone(),
                reason: e.to_string(),
            })?;

        // Record the migration as applied.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        tx.execute(
            "INSERT INTO schema_versions (version, applied_at, filename) VALUES (?1, ?2, ?3)",
            rusqlite::params![version, now, filename],
        )
        .map_err(|e| MigrationError::SqlExecutionFailed {
            file: filename.clone(),
            reason: e.to_string(),
        })?;

        tx.commit()
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: filename.clone(),
                reason: e.to_string(),
            })?;

        newly_applied.push(filename.clone());
    }

    Ok(newly_applied)
}

/// Embedded migration SQL files compiled into the binary.
/// This ensures migrations work regardless of the working directory.
static EMBEDDED_MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_initial_schema.sql",
        include_str!("../../migrations/0001_initial_schema.sql"),
    ),
    (
        "0002_security_tables.sql",
        include_str!("../../migrations/0002_security_tables.sql"),
    ),
    (
        "0003_memory_tables.sql",
        include_str!("../../migrations/0003_memory_tables.sql"),
    ),
    (
        "0004_fts5_index.sql",
        include_str!("../../migrations/0004_fts5_index.sql"),
    ),
    (
        "0005_vector_index.sql",
        include_str!("../../migrations/0005_vector_index.sql"),
    ),
    (
        "0006_add_document_kind.sql",
        include_str!("../../migrations/0006_add_document_kind.sql"),
    ),
    (
        "0007_token_savings.sql",
        include_str!("../../migrations/0007_token_savings.sql"),
    ),
    (
        "0008_add_method_kind.sql",
        include_str!("../../migrations/0008_add_method_kind.sql"),
    ),
];

/// Run embedded migrations that are compiled into the binary.
///
/// This is the preferred entry point - it doesn't require a `migrations/` directory
/// on disk. Falls back to `run_migrations()` if the embedded list is empty.
pub fn run_embedded_migrations(conn: &Connection) -> Result<Vec<String>, MigrationError> {
    ensure_schema_versions_table(conn)?;

    // Determine which migrations have already been applied.
    let applied: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT version FROM schema_versions ORDER BY version")
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: "<schema_versions query>".to_string(),
                reason: e.to_string(),
            })?;

        stmt.query_map([], |row| row.get(0))
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: "<schema_versions query>".to_string(),
                reason: e.to_string(),
            })?
            .filter_map(|r| r.ok())
            .collect()
    };

    let mut newly_applied: Vec<String> = Vec::new();

    for (filename, sql) in EMBEDDED_MIGRATIONS {
        let version = match parse_version(filename) {
            Some(v) => v,
            None => continue,
        };

        if applied.contains(&version) {
            continue;
        }

        // Execute the migration in its own transaction.
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: filename.to_string(),
                reason: e.to_string(),
            })?;

        tx.execute_batch(sql)
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: filename.to_string(),
                reason: e.to_string(),
            })?;

        // Record the migration as applied.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        tx.execute(
            "INSERT INTO schema_versions (version, applied_at, filename) VALUES (?1, ?2, ?3)",
            rusqlite::params![version, now, filename],
        )
        .map_err(|e| MigrationError::SqlExecutionFailed {
            file: filename.to_string(),
            reason: e.to_string(),
        })?;

        tx.commit()
            .map_err(|e| MigrationError::SqlExecutionFailed {
                file: filename.to_string(),
                reason: e.to_string(),
            })?;

        newly_applied.push(filename.to_string());
    }

    Ok(newly_applied)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;

    /// Helper: create an in-memory connection and a temp directory with migration files.
    fn setup_test_env(migrations: &[(&str, &str)]) -> (Connection, tempfile::TempDir) {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        let tmp = tempfile::tempdir().expect("failed to create temp dir");

        for (filename, content) in migrations {
            fs::write(tmp.path().join(filename), content).expect("failed to write migration file");
        }

        (conn, tmp)
    }

    #[test]
    fn test_apply_migrations_creates_tables() {
        let migrations = &[
            (
                "0001_create_users.sql",
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
            ),
            (
                "0002_create_posts.sql",
                "CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER, title TEXT);",
            ),
        ];

        let (conn, tmp) = setup_test_env(migrations);
        let applied = run_migrations(&conn, tmp.path()).expect("migrations should succeed");

        assert_eq!(applied.len(), 2);
        assert_eq!(applied[0], "0001_create_users.sql");
        assert_eq!(applied[1], "0002_create_posts.sql");

        // Verify tables exist.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='users'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='posts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_idempotent_second_run() {
        let migrations = &[(
            "0001_create_items.sql",
            "CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT);",
        )];

        let (conn, tmp) = setup_test_env(migrations);

        // First run.
        let applied = run_migrations(&conn, tmp.path()).expect("first run should succeed");
        assert_eq!(applied.len(), 1);

        // Second run - should apply nothing.
        let applied = run_migrations(&conn, tmp.path()).expect("second run should succeed");
        assert_eq!(applied.len(), 0);

        // Verify schema_versions has exactly one entry.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_versions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_failure_names_the_file() {
        let migrations = &[
            (
                "0001_good.sql",
                "CREATE TABLE good_table (id INTEGER PRIMARY KEY);",
            ),
            ("0002_bad.sql", "THIS IS NOT VALID SQL;"),
        ];

        let (conn, tmp) = setup_test_env(migrations);
        let result = run_migrations(&conn, tmp.path());

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("0002_bad.sql"),
            "Error should name the failed file, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_failed_migration_does_not_apply_partially() {
        let migrations = &[
            (
                "0001_good.sql",
                "CREATE TABLE alpha (id INTEGER PRIMARY KEY);",
            ),
            (
                "0002_bad.sql",
                "CREATE TABLE beta (id INTEGER PRIMARY KEY); THIS IS INVALID;",
            ),
        ];

        let (conn, tmp) = setup_test_env(migrations);
        let result = run_migrations(&conn, tmp.path());

        assert!(result.is_err());

        // First migration should have been applied.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='alpha'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Second migration should have been rolled back - beta should not exist.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='beta'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_numeric_ordering() {
        let migrations = &[
            (
                "0010_tenth.sql",
                "CREATE TABLE tenth (id INTEGER PRIMARY KEY);",
            ),
            (
                "0002_second.sql",
                "CREATE TABLE second (id INTEGER PRIMARY KEY);",
            ),
            (
                "0001_first.sql",
                "CREATE TABLE first (id INTEGER PRIMARY KEY);",
            ),
        ];

        let (conn, tmp) = setup_test_env(migrations);
        let applied = run_migrations(&conn, tmp.path()).expect("migrations should succeed");

        assert_eq!(applied.len(), 3);
        assert_eq!(applied[0], "0001_first.sql");
        assert_eq!(applied[1], "0002_second.sql");
        assert_eq!(applied[2], "0010_tenth.sql");
    }

    #[test]
    fn test_non_sql_files_ignored() {
        let migrations = &[
            (
                "0001_create.sql",
                "CREATE TABLE things (id INTEGER PRIMARY KEY);",
            ),
            ("README.md", "# Migrations\nDo not edit released files."),
        ];

        let (conn, tmp) = setup_test_env(migrations);
        let applied = run_migrations(&conn, tmp.path()).expect("migrations should succeed");

        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0], "0001_create.sql");
    }

    #[test]
    fn test_missing_directory_returns_error() {
        let conn = Connection::open_in_memory().expect("failed to open in-memory db");
        let result = run_migrations(&conn, Path::new("/nonexistent/path/migrations"));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, MigrationError::FileReadFailed { .. }));
    }

    #[test]
    fn test_schema_versions_records_metadata() {
        let migrations = &[(
            "0001_init.sql",
            "CREATE TABLE init_table (id INTEGER PRIMARY KEY);",
        )];

        let (conn, tmp) = setup_test_env(migrations);
        run_migrations(&conn, tmp.path()).expect("migrations should succeed");

        let (version, filename): (i64, String) = conn
            .query_row(
                "SELECT version, filename FROM schema_versions WHERE version = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("should find schema_versions entry");

        assert_eq!(version, 1);
        assert_eq!(filename, "0001_init.sql");

        // applied_at should be a reasonable timestamp (after year 2020).
        let applied_at: i64 = conn
            .query_row(
                "SELECT applied_at FROM schema_versions WHERE version = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(applied_at > 1_577_836_800); // 2020-01-01
    }
}
