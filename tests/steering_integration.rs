//! Integration tests for the install-time steering file generation flow.
//!
//! Tests verify:
//! - Steering file is created in the correct location for each agent
//! - Steering file contains Cortex markers and tool recommendations
//! - Writing the steering file twice is idempotent (no duplication)
//! - Fallback path is used when no known agent directory exists
//!
//! _Requirements: 8.1, 8.2, 8.8, 8.9_

use std::fs;
use tempfile::TempDir;

use cortex::agents::steering::{
    STEERING_TEMPLATE, contains_cortex_content, steering_file_path, write_steering_file,
};

/// Helper: create a temp directory simulating a repo root.
fn setup_repo() -> TempDir {
    TempDir::new().expect("failed to create temp directory")
}

// ---------------------------------------------------------------------------
// Test: Cursor agent - .cursor/rules/cortex.mdc is created
// ---------------------------------------------------------------------------

#[test]
fn test_cursor_steering_file_created_with_correct_content() {
    let tmp = setup_repo();
    let root = tmp.path();

    // Simulate a repo with .cursor/ directory present
    fs::create_dir_all(root.join(".cursor")).unwrap();

    // Write the steering file for Cursor
    write_steering_file("cursor", root, STEERING_TEMPLATE).unwrap();

    // Verify .cursor/rules/cortex.mdc is created
    let path = root.join(".cursor").join("rules").join("cortex.mdc");
    assert!(path.exists(), "Expected .cursor/rules/cortex.mdc to exist");

    // Verify content has Cortex steering markers
    let content = fs::read_to_string(&path).unwrap();
    assert!(
        contains_cortex_content(&content),
        "File should contain Cortex steering markers"
    );

    // Verify content includes tool recommendations
    assert!(
        content.contains("ask"),
        "Steering file should recommend 'ask' tool"
    );
    assert!(
        content.contains("search_symbols"),
        "Steering file should recommend 'search_symbols' tool"
    );
    assert!(
        content.contains("trace_callers"),
        "Steering file should recommend 'trace_callers' tool"
    );
    assert!(
        content.contains("get_file_context"),
        "Steering file should recommend 'get_file_context' tool"
    );
    assert!(
        content.contains("blast_radius"),
        "Steering file should recommend 'blast_radius' tool"
    );
    assert!(
        content.contains("token"),
        "Steering file should mention token savings benefit"
    );
}

// ---------------------------------------------------------------------------
// Test: Idempotency - writing twice produces identical content
// ---------------------------------------------------------------------------

#[test]
fn test_cursor_steering_file_idempotent_no_duplication() {
    let tmp = setup_repo();
    let root = tmp.path();

    // Create .cursor/ directory
    fs::create_dir_all(root.join(".cursor")).unwrap();

    // Write steering file first time
    write_steering_file("cursor", root, STEERING_TEMPLATE).unwrap();
    let path = steering_file_path("cursor", root);
    let first_content = fs::read_to_string(&path).unwrap();

    // Write steering file second time (should be idempotent)
    write_steering_file("cursor", root, STEERING_TEMPLATE).unwrap();
    let second_content = fs::read_to_string(&path).unwrap();

    // Content should be identical
    assert_eq!(
        first_content, second_content,
        "Writing steering file twice should produce identical content (idempotency)"
    );

    // Markers should appear exactly once
    let start_count = second_content
        .matches("<!-- cortex-steering-start -->")
        .count();
    let end_count = second_content
        .matches("<!-- cortex-steering-end -->")
        .count();
    assert_eq!(start_count, 1, "Start marker should appear exactly once");
    assert_eq!(end_count, 1, "End marker should appear exactly once");
}

// ---------------------------------------------------------------------------
// Test: Fallback path when no known agent directory exists
// ---------------------------------------------------------------------------

#[test]
fn test_fallback_steering_file_when_no_agent_directory() {
    let tmp = setup_repo();
    let root = tmp.path();

    // No .cursor/, .claude/, .kiro/, etc. directories exist
    // Use an unknown agent name to trigger fallback
    write_steering_file("unknown", root, STEERING_TEMPLATE).unwrap();

    // Verify .cortex/steering.md is created (fallback path)
    let path = root.join(".cortex").join("steering.md");
    assert!(
        path.exists(),
        "Expected .cortex/steering.md to exist as fallback"
    );

    let content = fs::read_to_string(&path).unwrap();
    assert!(
        contains_cortex_content(&content),
        "Fallback file should contain Cortex steering markers"
    );
    assert!(
        content.contains("Cortex MCP Tools"),
        "Fallback file should contain tool recommendations"
    );
}

// ---------------------------------------------------------------------------
// Test: Claude Code agent - .claude/CLAUDE.md is created
// ---------------------------------------------------------------------------

#[test]
fn test_claude_steering_file_created() {
    let tmp = setup_repo();
    let root = tmp.path();

    // Simulate a repo with .claude/ directory present
    fs::create_dir_all(root.join(".claude")).unwrap();

    write_steering_file("claude code", root, STEERING_TEMPLATE).unwrap();

    // Verify .claude/CLAUDE.md is created
    let path = root.join(".claude").join("CLAUDE.md");
    assert!(path.exists(), "Expected .claude/CLAUDE.md to exist");

    let content = fs::read_to_string(&path).unwrap();
    assert!(contains_cortex_content(&content));
    assert!(content.contains("Cortex MCP Tools"));
}

// ---------------------------------------------------------------------------
// Test: Kiro agent - .kiro/steering/cortex.md is created
// ---------------------------------------------------------------------------

#[test]
fn test_kiro_steering_file_created() {
    let tmp = setup_repo();
    let root = tmp.path();

    // Simulate a repo with .kiro/ directory present
    fs::create_dir_all(root.join(".kiro")).unwrap();

    write_steering_file("kiro", root, STEERING_TEMPLATE).unwrap();

    // Verify .kiro/steering/cortex.md is created
    let path = root.join(".kiro").join("steering").join("cortex.md");
    assert!(path.exists(), "Expected .kiro/steering/cortex.md to exist");

    let content = fs::read_to_string(&path).unwrap();
    assert!(contains_cortex_content(&content));
    assert!(content.contains("Cortex MCP Tools"));
}

// ---------------------------------------------------------------------------
// Test: Steering file path mapping is correct for all agents
// ---------------------------------------------------------------------------

#[test]
fn test_steering_file_path_mapping() {
    let root = std::path::Path::new("/repo");

    assert_eq!(
        steering_file_path("cursor", root),
        root.join(".cursor").join("rules").join("cortex.mdc")
    );
    assert_eq!(
        steering_file_path("claude code", root),
        root.join(".claude").join("CLAUDE.md")
    );
    assert_eq!(
        steering_file_path("claude", root),
        root.join(".claude").join("CLAUDE.md")
    );
    assert_eq!(
        steering_file_path("kiro", root),
        root.join(".kiro").join("steering").join("cortex.md")
    );
    assert_eq!(
        steering_file_path("windsurf", root),
        root.join(".windsurfrules")
    );
    assert_eq!(
        steering_file_path("copilot", root),
        root.join(".github").join("copilot-instructions.md")
    );
    assert_eq!(
        steering_file_path("anything-else", root),
        root.join(".cortex").join("steering.md")
    );
}

// ---------------------------------------------------------------------------
// Test: Append to existing file without Cortex content
// ---------------------------------------------------------------------------

#[test]
fn test_steering_appends_to_existing_file_without_cortex_content() {
    let tmp = setup_repo();
    let root = tmp.path();

    // Create an existing .claude/CLAUDE.md with non-Cortex content
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    let path = claude_dir.join("CLAUDE.md");
    fs::write(
        &path,
        "# Existing Project Rules\n\nDo not modify generated files.\n",
    )
    .unwrap();

    write_steering_file("claude code", root, STEERING_TEMPLATE).unwrap();

    let content = fs::read_to_string(&path).unwrap();

    // Should preserve existing content
    assert!(
        content.contains("Existing Project Rules"),
        "Should preserve existing content"
    );
    assert!(
        content.contains("Do not modify generated files."),
        "Should preserve existing rules"
    );

    // Should also have Cortex content appended
    assert!(
        contains_cortex_content(&content),
        "Should have Cortex content appended"
    );
}

// ---------------------------------------------------------------------------
// Test: Replace existing Cortex section without duplicating
// ---------------------------------------------------------------------------

#[test]
fn test_steering_replaces_existing_cortex_section() {
    let tmp = setup_repo();
    let root = tmp.path();

    let windsurf_path = root.join(".windsurfrules");
    let existing = "# My Rules\n\n<!-- cortex-steering-start -->\nOld Cortex content here\n<!-- cortex-steering-end -->\n\n# Footer\n";
    fs::write(&windsurf_path, existing).unwrap();

    write_steering_file("windsurf", root, STEERING_TEMPLATE).unwrap();

    let content = fs::read_to_string(&windsurf_path).unwrap();

    // Old content should be gone
    assert!(
        !content.contains("Old Cortex content here"),
        "Old Cortex content should be replaced"
    );

    // New template content should be present
    assert!(
        content.contains("Cortex MCP Tools"),
        "New template content should be present"
    );

    // Surrounding content should be preserved
    assert!(content.contains("# My Rules"), "Header should be preserved");
    assert!(content.contains("# Footer"), "Footer should be preserved");

    // No duplication of markers
    let start_count = content.matches("<!-- cortex-steering-start -->").count();
    let end_count = content.matches("<!-- cortex-steering-end -->").count();
    assert_eq!(start_count, 1, "Start marker should appear exactly once");
    assert_eq!(end_count, 1, "End marker should appear exactly once");
}

// ---------------------------------------------------------------------------
// Test: Idempotency with existing non-Cortex content
// ---------------------------------------------------------------------------

#[test]
fn test_steering_idempotent_with_existing_content() {
    let tmp = setup_repo();
    let root = tmp.path();

    // Create existing file
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    let path = claude_dir.join("CLAUDE.md");
    fs::write(&path, "# Project\n\nExisting content.\n").unwrap();

    // Write once
    write_steering_file("claude code", root, STEERING_TEMPLATE).unwrap();
    let first_content = fs::read_to_string(&path).unwrap();

    // Write again
    write_steering_file("claude code", root, STEERING_TEMPLATE).unwrap();
    let second_content = fs::read_to_string(&path).unwrap();

    assert_eq!(
        first_content, second_content,
        "Second write should not change content (idempotent)"
    );
}

// ---------------------------------------------------------------------------
// Test: contains_cortex_content detection
// ---------------------------------------------------------------------------

#[test]
fn test_contains_cortex_content_detection() {
    // Both markers present
    let with_markers =
        "before\n<!-- cortex-steering-start -->\ncontent\n<!-- cortex-steering-end -->\nafter";
    assert!(contains_cortex_content(with_markers));

    // No markers
    let without_markers = "just some regular content";
    assert!(!contains_cortex_content(without_markers));

    // Only start marker
    let only_start = "<!-- cortex-steering-start -->\ncontent without end";
    assert!(!contains_cortex_content(only_start));

    // Only end marker
    let only_end = "content without start\n<!-- cortex-steering-end -->";
    assert!(!contains_cortex_content(only_end));

    // Template itself should contain markers
    assert!(contains_cortex_content(STEERING_TEMPLATE));
}
