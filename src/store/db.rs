//! Database connection manager for the Cortex graph store.
//!
//! Manages one exclusive write connection and a configurable pool of read-only
//! connections (default 4, max 16). All connections are configured with WAL mode,
//! synchronous=NORMAL, and foreign_keys=ON for optimal concurrent read
//! performance with safe writes.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::StoreError;

/// Number of read-only connections in the pool (default).
const DEFAULT_READ_POOL_SIZE: usize = 4;

/// Database file name within the data directory.
const DB_FILENAME: &str = "graph.db";

/// Manages SQLite connections for the graph store.
///
/// Holds a single write connection protected by a mutex and a pool of
/// read-only connections for concurrent query access. WAL mode enables
/// readers to proceed without blocking on the writer.
pub struct StoreManager {
    write_conn: Mutex<Connection>,
    read_pool: Vec<Mutex<Connection>>,
}

impl StoreManager {
    /// Creates a new `StoreManager` with the database at `{data_dir}/graph.db`.
    ///
    /// Opens one write connection and `pool_size` read-only connections,
    /// applying required PRAGMAs to each. The pool_size defaults to 4 if not
    /// specified (pass 0 or use `new_default` for the default).
    /// Values above 16 are clamped to 16.
    pub fn new(data_dir: &Path) -> Result<Self, StoreError> {
        Self::with_pool_size(data_dir, DEFAULT_READ_POOL_SIZE)
    }

    /// Creates a new `StoreManager` with a configurable read pool size.
    ///
    /// `pool_size` is clamped to a minimum of 1 and maximum of 16.
    pub fn with_pool_size(data_dir: &Path, pool_size: usize) -> Result<Self, StoreError> {
        let pool_size = pool_size.clamp(1, 16);

        std::fs::create_dir_all(data_dir).map_err(|e| StoreError::ConnectionFailed {
            reason: format!(
                "failed to create data directory '{}': {}",
                data_dir.display(),
                e
            ),
        })?;

        let db_path = data_dir.join(DB_FILENAME);

        let write_conn = open_connection(&db_path)?;
        apply_pragmas(&write_conn)?;

        let mut read_pool = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let conn = open_connection(&db_path)?;
            apply_pragmas(&conn)?;
            read_pool.push(Mutex::new(conn));
        }

        Ok(Self {
            write_conn: Mutex::new(write_conn),
            read_pool,
        })
    }

    /// Returns a mutex guard to the exclusive write connection.
    pub fn write_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.write_conn
            .lock()
            .expect("write connection mutex poisoned")
    }

    /// Returns a mutex guard to a pooled read-only connection.
    ///
    /// Uses a simple round-robin approach by trying each connection in order
    /// and returning the first one that is not currently locked. Falls back to
    /// blocking on the first connection if all are busy.
    pub fn read_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // Try to acquire a reader without blocking first.
        for reader in &self.read_pool {
            if let Ok(guard) = reader.try_lock() {
                return guard;
            }
        }
        // All readers busy - block on the first one.
        self.read_pool[0]
            .lock()
            .expect("read connection mutex poisoned")
    }
}

/// Opens a new SQLite connection at the given path.
fn open_connection(path: &Path) -> Result<Connection, StoreError> {
    Connection::open(path).map_err(|e| StoreError::ConnectionFailed {
        reason: format!("failed to open database '{}': {}", path.display(), e),
    })
}

/// Applies required PRAGMAs to a connection:
/// - journal_mode = WAL (write-ahead logging for concurrent reads)
/// - synchronous = NORMAL (safe with WAL, better performance than FULL)
/// - foreign_keys = ON (enforce referential integrity)
fn apply_pragmas(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )
    .map_err(|e| StoreError::ConnectionFailed {
        reason: format!("failed to apply PRAGMAs: {}", e),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    /// Helper to create a StoreManager with a temporary directory.
    fn create_temp_store() -> (StoreManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = StoreManager::new(tmp.path()).expect("failed to create StoreManager");
        (store, tmp)
    }

    #[test]
    fn test_connection_opens_successfully() {
        let (store, _tmp) = create_temp_store();
        // Verify we can acquire both write and read connections.
        let _write = store.write_conn();
        let _read = store.read_conn();
    }

    #[test]
    fn test_database_file_created() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let _store = StoreManager::new(tmp.path()).expect("failed to create StoreManager");
        assert!(tmp.path().join(DB_FILENAME).exists());
    }

    #[test]
    fn test_pragmas_applied_on_write_connection() {
        let (store, _tmp) = create_temp_store();
        let conn = store.write_conn();

        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .expect("failed to query journal_mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let synchronous: i32 = conn
            .query_row("PRAGMA synchronous;", [], |row| row.get(0))
            .expect("failed to query synchronous");
        // NORMAL = 1
        assert_eq!(synchronous, 1);

        let foreign_keys: i32 = conn
            .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
            .expect("failed to query foreign_keys");
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn test_pragmas_applied_on_read_connections() {
        let (store, _tmp) = create_temp_store();
        let conn = store.read_conn();

        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .expect("failed to query journal_mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let synchronous: i32 = conn
            .query_row("PRAGMA synchronous;", [], |row| row.get(0))
            .expect("failed to query synchronous");
        assert_eq!(synchronous, 1);

        let foreign_keys: i32 = conn
            .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
            .expect("failed to query foreign_keys");
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn test_concurrent_reads() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = Arc::new(StoreManager::new(tmp.path()).expect("failed to create StoreManager"));

        // Create a table via the write connection for readers to query.
        {
            let conn = store.write_conn();
            conn.execute_batch("CREATE TABLE test_table (id INTEGER PRIMARY KEY, value TEXT);")
                .expect("failed to create table");
            conn.execute(
                "INSERT INTO test_table (id, value) VALUES (1, 'hello');",
                [],
            )
            .expect("failed to insert");
        }

        // Spawn multiple reader threads that all query concurrently.
        let mut handles = Vec::new();
        for _ in 0..DEFAULT_READ_POOL_SIZE {
            let store_clone = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                let conn = store_clone.read_conn();
                let value: String = conn
                    .query_row("SELECT value FROM test_table WHERE id = 1;", [], |row| {
                        row.get(0)
                    })
                    .expect("failed to query");
                assert_eq!(value, "hello");
            }));
        }

        for handle in handles {
            handle.join().expect("reader thread panicked");
        }
    }

    #[test]
    fn test_exclusive_writes() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = Arc::new(StoreManager::new(tmp.path()).expect("failed to create StoreManager"));

        // Create a table for the test.
        {
            let conn = store.write_conn();
            conn.execute_batch(
                "CREATE TABLE counter (id INTEGER PRIMARY KEY, count INTEGER NOT NULL);
                 INSERT INTO counter (id, count) VALUES (1, 0);",
            )
            .expect("failed to create counter table");
        }

        // Spawn multiple writer threads that each increment the counter.
        let num_writers = 8;
        let mut handles = Vec::new();
        for _ in 0..num_writers {
            let store_clone = Arc::clone(&store);
            handles.push(thread::spawn(move || {
                let conn = store_clone.write_conn();
                conn.execute("UPDATE counter SET count = count + 1 WHERE id = 1;", [])
                    .expect("failed to update counter");
            }));
        }

        for handle in handles {
            handle.join().expect("writer thread panicked");
        }

        // Verify all writes were serialized correctly.
        let conn = store.write_conn();
        let count: i32 = conn
            .query_row("SELECT count FROM counter WHERE id = 1;", [], |row| {
                row.get(0)
            })
            .expect("failed to query counter");
        assert_eq!(count, num_writers);
    }

    #[test]
    fn test_read_pool_has_four_connections() {
        let (store, _tmp) = create_temp_store();
        assert_eq!(store.read_pool.len(), DEFAULT_READ_POOL_SIZE);
    }

    #[test]
    fn test_new_with_nonexistent_data_dir() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let nested = tmp.path().join("deeply").join("nested").join("dir");
        let store = StoreManager::new(&nested);
        assert!(store.is_ok());
        assert!(nested.join(DB_FILENAME).exists());
    }

    #[test]
    fn test_pool_size_8_creates_8_readers() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store =
            StoreManager::with_pool_size(tmp.path(), 8).expect("failed to create StoreManager");
        assert_eq!(store.read_pool.len(), 8);
    }

    #[test]
    fn test_pool_size_clamped_to_max_16() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store =
            StoreManager::with_pool_size(tmp.path(), 32).expect("failed to create StoreManager");
        assert_eq!(store.read_pool.len(), 16);
    }

    #[test]
    fn test_pool_size_clamped_to_min_1() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store =
            StoreManager::with_pool_size(tmp.path(), 0).expect("failed to create StoreManager");
        assert_eq!(store.read_pool.len(), 1);
    }
}
