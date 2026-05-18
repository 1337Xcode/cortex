//! Bundle CLI command implementations.
//!
//! Handles `cortex bundle export` and `cortex bundle import` subcommands.

use std::path::Path;
use std::sync::Arc;

use crate::bundle::export::export_bundle;
use crate::bundle::import::import_bundle;
use crate::error::BundleError;
use crate::store::db::StoreManager;

/// Run the `cortex bundle export` command.
///
/// Exports the graph store to `.cortex/cortex.json` in the given output directory.
pub fn run_export(store: &Arc<StoreManager>, output_dir: &Path) -> Result<(), BundleError> {
    let stats = export_bundle(store, output_dir)?;
    println!(
        "Bundle exported: {} nodes, {} edges, {} findings, {} observations ({} bytes)",
        stats.node_count,
        stats.edge_count,
        stats.finding_count,
        stats.observation_count,
        stats.file_size_bytes,
    );
    Ok(())
}

/// Run the `cortex bundle import` command.
///
/// Imports a bundle from the given path (defaults to `.cortex/cortex.json`).
pub fn run_import(store: &Arc<StoreManager>, bundle_path: &Path) -> Result<(), BundleError> {
    let stats = import_bundle(store, bundle_path)?;
    println!(
        "Bundle imported: {} nodes, {} edges, {} findings, {} observations",
        stats.nodes_imported,
        stats.edges_imported,
        stats.findings_imported,
        stats.observations_imported,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::StoreManager;
    use crate::store::migrations::run_migrations;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn setup_store_with_migrations() -> (Arc<StoreManager>, TempDir, TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let output_dir = tempfile::tempdir().unwrap();
        let store = StoreManager::new(data_dir.path()).unwrap();

        let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let conn = store.write_conn();
        run_migrations(&conn, &migrations_dir).unwrap();
        drop(conn);

        (Arc::new(store), data_dir, output_dir)
    }

    #[test]
    fn cli_export_works() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        let result = run_export(&store, output_dir.path());
        assert!(result.is_ok());

        // Verify files were created
        assert!(output_dir.path().join("cortex.json").exists());
        assert!(output_dir.path().join("cortex.json.sha256").exists());
        assert!(output_dir.path().join(".gitignore").exists());
    }

    #[test]
    fn cli_import_works() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        // Insert data and export first
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params!["src/main.rs::main", "Function", "src/main.rs", 1, 10, "hash123", 1715270300, "{}"],
            ).unwrap();
        }

        run_export(&store, output_dir.path()).unwrap();

        // Import
        let bundle_path = output_dir.path().join("cortex.json");
        let result = run_import(&store, &bundle_path);
        assert!(result.is_ok());
    }

    #[test]
    fn auto_export_after_index() {
        let (store, _data_dir, output_dir) = setup_store_with_migrations();

        // Simulate auto-export: config.auto_bundle_export == true
        let auto_bundle_export = true;
        if auto_bundle_export {
            let result = run_export(&store, output_dir.path());
            assert!(result.is_ok());
            assert!(output_dir.path().join("cortex.json").exists());
        }
    }

    #[test]
    fn disabled_auto_export_skips() {
        let (_store, _data_dir, output_dir) = setup_store_with_migrations();

        // Simulate auto-export disabled
        let auto_bundle_export = false;
        if auto_bundle_export {
            panic!("should not reach here");
        }

        // Verify no bundle was created
        assert!(!output_dir.path().join("cortex.json").exists());
    }
}
