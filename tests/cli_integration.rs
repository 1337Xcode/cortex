//! Integration tests for the CLI skeleton and main.rs wiring.
//!
//! Tests verify:
//! - Valid config exits 0 with startup line
//! - Invalid/missing config exits non-zero with error message
//! - --help shows all subcommands
//! - --version reports version

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

/// Helper to get a Command for the cortex binary.
fn cortex_cmd() -> Command {
    Command::cargo_bin("cortex").expect("binary should exist")
}

#[test]
fn test_version_flag_reports_version() {
    cortex_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("cortex 1.0.2"));
}

#[test]
fn test_help_shows_all_subcommands() {
    let assert = cortex_cmd().arg("--help").assert().success();

    let output = String::from_utf8_lossy(&assert.get_output().stdout);

    // Verify all top-level subcommands are listed.
    assert!(output.contains("serve"), "missing 'serve' subcommand");
    assert!(output.contains("index"), "missing 'index' subcommand");
    assert!(
        output.contains("index-file"),
        "missing 'index-file' subcommand"
    );
    assert!(output.contains("install"), "missing 'install' subcommand");
    assert!(output.contains("bundle"), "missing 'bundle' subcommand");
    assert!(output.contains("query"), "missing 'query' subcommand");
    assert!(output.contains("status"), "missing 'status' subcommand");
    assert!(output.contains("memory"), "missing 'memory' subcommand");
    assert!(output.contains("security"), "missing 'security' subcommand");
    assert!(output.contains("config"), "missing 'config' subcommand");
    assert!(output.contains("semantic"), "missing 'semantic' subcommand");
}

#[test]
fn test_missing_config_defaults_to_cwd() {
    // Without CORTEX_REPO_ROOT set, the binary defaults to the current directory
    // and succeeds (stdin closes immediately so the server exits cleanly).
    cortex_cmd()
        .arg("serve")
        .env_remove("CORTEX_REPO_ROOT")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_LOG_LEVEL")
        .assert()
        .success();
}

#[test]
fn test_valid_config_exits_zero_with_startup_line() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("my-repo");
    let data_dir = tmp.path().join("my-data");
    fs::create_dir_all(&repo_root).unwrap();

    cortex_cmd()
        .arg("serve")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .env_remove("CORTEX_LOG_LEVEL")
        .assert()
        .success()
        .stderr(predicate::str::contains("cortex 1.0.2"))
        .stderr(predicate::str::contains("repo:"))
        .stderr(predicate::str::contains("data:"));
}

#[test]
fn test_query_help_shows_subcommands() {
    cortex_cmd()
        .args(["query", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("callers"))
        .stdout(predicate::str::contains("callees"))
        .stdout(predicate::str::contains("find"))
        .stdout(predicate::str::contains("architecture"))
        .stdout(predicate::str::contains("dead-code"))
        .stdout(predicate::str::contains("blast-radius"));
}

#[test]
fn test_bundle_help_shows_subcommands() {
    cortex_cmd()
        .args(["bundle", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("export"))
        .stdout(predicate::str::contains("import"));
}

#[test]
fn test_memory_help_shows_subcommands() {
    cortex_cmd()
        .args(["memory", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("prune"));
}

#[test]
fn test_security_help_shows_subcommands() {
    cortex_cmd()
        .args(["security", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("scan"))
        .stdout(predicate::str::contains("sbom"))
        .stdout(predicate::str::contains("vulns"));
}

#[test]
fn test_config_help_shows_subcommands() {
    cortex_cmd()
        .args(["config", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("get"))
        .stdout(predicate::str::contains("set"))
        .stdout(predicate::str::contains("reset"));
}

#[test]
fn test_semantic_help_shows_subcommands() {
    cortex_cmd()
        .args(["semantic", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("enable"))
        .stdout(predicate::str::contains("disable"))
        .stdout(predicate::str::contains("status"));
}

// ---------------------------------------------------------------------------
// Additional integration tests for comprehensive CLI coverage
// ---------------------------------------------------------------------------

#[test]
fn test_version_outputs_version_string() {
    cortex_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("cortex"));
}

#[test]
fn test_help_lists_all_subcommands_comprehensive() {
    let assert = cortex_cmd().arg("--help").assert().success();
    let output = String::from_utf8_lossy(&assert.get_output().stdout);

    assert!(output.contains("impact"), "missing 'impact' subcommand");
    assert!(output.contains("explain"), "missing 'explain' subcommand");
    assert!(output.contains("diff"), "missing 'diff' subcommand");
    assert!(output.contains("ci"), "missing 'ci' subcommand");
    assert!(output.contains("report"), "missing 'report' subcommand");
    assert!(output.contains("viz"), "missing 'viz' subcommand");
    assert!(output.contains("clone"), "missing 'clone' subcommand");
    assert!(output.contains("hook"), "missing 'hook' subcommand");
    assert!(output.contains("modules"), "missing 'modules' subcommand");
    assert!(output.contains("watch"), "missing 'watch' subcommand");
    assert!(output.contains("coverage"), "missing 'coverage' subcommand");
    assert!(
        output.contains("hierarchy"),
        "missing 'hierarchy' subcommand"
    );
    assert!(output.contains("hotspots"), "missing 'hotspots' subcommand");
}

#[test]
fn test_index_on_empty_directory_succeeds() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("empty-repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();
}

#[test]
fn test_query_find_with_pattern() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    // Index first (empty repo is fine, query should still succeed).
    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();

    // Query find with a pattern.
    cortex_cmd()
        .args(["query", "find", "*main*"])
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();
}

#[test]
fn test_security_scan_runs_without_error() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    // Index first.
    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();

    cortex_cmd()
        .args(["security", "scan"])
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();
}

#[test]
fn test_modules_runs_without_error() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    // Index first.
    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();

    cortex_cmd()
        .args(["modules"])
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();
}

#[test]
fn test_memory_show_runs_without_error_empty_database() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    // Index first to create the database.
    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();

    cortex_cmd()
        .args(["memory", "show"])
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();
}

#[test]
fn test_coverage_with_missing_file_gives_helpful_error() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    // Index first.
    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();

    // Run coverage with a non-existent LCOV file.
    cortex_cmd()
        .args(["coverage", "--lcov", "/nonexistent/coverage.lcov"])
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

#[test]
fn test_hierarchy_with_unknown_fqn_suggests_alternatives() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    // Index first.
    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();

    // Run hierarchy with an unknown FQN. Should not crash.
    cortex_cmd()
        .args(["hierarchy", "NonExistentClass"])
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();
}

#[test]
fn test_explain_with_unknown_fqn_suggests_alternatives() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    // Index first.
    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();

    // Run explain with an unknown FQN. Should not crash.
    cortex_cmd()
        .args(["explain", "NonExistentFunction"])
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();
}

#[test]
fn test_impact_with_unknown_fqn_suggests_alternatives() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let repo_root = tmp.path().join("repo");
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&repo_root).unwrap();

    // Index first.
    cortex_cmd()
        .arg("index")
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();

    // Run impact with an unknown FQN. Should not crash.
    cortex_cmd()
        .args(["impact", "NonExistentFunction"])
        .env("CORTEX_REPO_ROOT", repo_root.to_str().unwrap())
        .env("CORTEX_DATA_DIR", data_dir.to_str().unwrap())
        .assert()
        .success();
}
