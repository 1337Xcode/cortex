//! Steering file generation: analyzes the graph to produce agent-specific
//! context files (CLAUDE.md, AGENTS.md, .cursorrules).
//!
//! Also provides install-time steering file writing that places a Cortex MCP
//! tool preference guide into the appropriate agent-specific location.
//!
//! Content is generated from templates based on detected languages, frameworks,
//! module boundaries, and entry points. No LLM-generated content is used.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite;
use serde::Serialize;

use crate::error::AgentError;
use crate::store::db::StoreManager;
use crate::store::queries::community;
use crate::store::queries::memory;

// ---------------------------------------------------------------------------
// Constants: steering markers and content template
// ---------------------------------------------------------------------------

/// Start marker for Cortex steering content in agent files.
const CORTEX_STEERING_START: &str = "<!-- cortex-steering-start -->";
/// End marker for Cortex steering content in agent files.
const CORTEX_STEERING_END: &str = "<!-- cortex-steering-end -->";

/// Default steering file content template recommending Cortex MCP tools.
pub const STEERING_TEMPLATE: &str = r#"<!-- cortex-steering-start -->
## Cortex MCP Tools

Prefer Cortex MCP tools over reading raw source files. Cortex provides structural
queries that return focused, relevant code context while saving significant tokens.

**Preferred tools:**
- `ask` - Natural language code intelligence (single call, graph-guided retrieval)
- `search_symbols` - Find symbols by pattern (faster than grep, returns structure)
- `trace_callers` / `trace_callees` - Navigate the call graph
- `get_file_context` - Get structured file overview with symbols and relationships
- `blast_radius` - Understand impact of changes before making them

**Why:** Each Cortex tool call saves ~750 tokens per file touched compared to reading
raw source. The graph-guided retrieval ensures you get the most relevant context first.
<!-- cortex-steering-end -->"#;

// ---------------------------------------------------------------------------
// Install-time steering file functions (Requirement 8)
// ---------------------------------------------------------------------------

/// Determine the steering file path for a detected agent.
///
/// Maps agent names to their conventional steering/rules file locations:
/// - Cursor -> `.cursor/rules/cortex.mdc`
/// - Claude Code -> `.claude/CLAUDE.md`
/// - Kiro -> `.kiro/steering/cortex.md`
/// - Windsurf -> `.windsurfrules`
/// - Copilot -> `.github/copilot-instructions.md`
/// - Fallback (anything else) -> `.cortex/steering.md`
pub fn steering_file_path(agent_name: &str, repo_root: &Path) -> PathBuf {
    match agent_name.to_lowercase().as_str() {
        "cursor" => repo_root.join(".cursor").join("rules").join("cortex.mdc"),
        "claude code" | "claude" => repo_root.join(".claude").join("CLAUDE.md"),
        "kiro" => repo_root.join(".kiro").join("steering").join("cortex.md"),
        "windsurf" => repo_root.join(".windsurfrules"),
        "copilot" => repo_root.join(".github").join("copilot-instructions.md"),
        _ => repo_root.join(".cortex").join("steering.md"),
    }
}

/// Check if file content already contains Cortex steering content.
///
/// Looks for the presence of both the start and end markers:
/// `<!-- cortex-steering-start -->` and `<!-- cortex-steering-end -->`.
pub fn contains_cortex_content(file_content: &str) -> bool {
    file_content.contains(CORTEX_STEERING_START) && file_content.contains(CORTEX_STEERING_END)
}

/// Write or update the steering file for the given agent.
///
/// Behavior:
/// - If the file does not exist, creates parent directories and writes the content
///   (wrapped in cortex markers).
/// - If the file exists and already contains Cortex content (identified by markers),
///   replaces the existing Cortex section with the new content (idempotent update).
/// - If the file exists but does not contain Cortex content, appends the content.
///
/// The `content` parameter should be the full steering template including markers.
/// If it does not include markers, the default `STEERING_TEMPLATE` is used.
pub fn write_steering_file(
    agent_name: &str,
    repo_root: &Path,
    content: &str,
) -> Result<(), AgentError> {
    let path = steering_file_path(agent_name, repo_root);

    // Ensure the content has markers; use the template if not provided with markers
    let steering_content = if contains_cortex_content(content) {
        content.to_string()
    } else if content.is_empty() {
        STEERING_TEMPLATE.to_string()
    } else {
        format!(
            "{}\n{}\n{}",
            CORTEX_STEERING_START, content, CORTEX_STEERING_END
        )
    };

    // Create parent directories if they don't exist
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AgentError::ConfigurationFailed {
            reason: format!("failed to create directory '{}': {}", parent.display(), e),
        })?;
    }

    // Read existing file content if the file exists
    let existing_content = std::fs::read_to_string(&path).ok();

    let final_content = match existing_content {
        Some(existing) if contains_cortex_content(&existing) => {
            // Replace the existing Cortex section
            replace_cortex_section(&existing, &steering_content)
        }
        Some(existing) => {
            // Append to existing file with a newline separator
            if existing.ends_with('\n') {
                format!("{}\n{}\n", existing, steering_content)
            } else {
                format!("{}\n\n{}\n", existing, steering_content)
            }
        }
        None => {
            // New file: just write the steering content
            format!("{}\n", steering_content)
        }
    };

    std::fs::write(&path, final_content).map_err(|e| AgentError::ConfigurationFailed {
        reason: format!("failed to write steering file '{}': {}", path.display(), e),
    })?;

    Ok(())
}

/// Replace the Cortex steering section in existing file content.
///
/// Finds the region between (and including) the start and end markers and
/// replaces it with the new content.
fn replace_cortex_section(existing: &str, new_section: &str) -> String {
    if let (Some(start_idx), Some(end_idx)) = (
        existing.find(CORTEX_STEERING_START),
        existing.find(CORTEX_STEERING_END),
    ) {
        let end_of_marker = end_idx + CORTEX_STEERING_END.len();
        // Consume a trailing newline after the end marker if present
        let end_of_section = if existing[end_of_marker..].starts_with('\n') {
            end_of_marker + 1
        } else {
            end_of_marker
        };

        let before = &existing[..start_idx];
        let after = &existing[end_of_section..];

        format!("{}{}\n{}", before, new_section, after)
    } else {
        // Shouldn't happen since we checked contains_cortex_content, but handle gracefully
        format!("{}\n{}\n", existing, new_section)
    }
}

/// Content generated for steering files.
#[derive(Debug, Clone, Serialize)]
pub struct SteeringContent {
    /// Content for CLAUDE.md
    pub claude_md: String,
    /// Content for AGENTS.md
    pub agents_md: String,
    /// Content for .cursorrules
    pub cursorrules: String,
    /// Detected languages
    pub languages: Vec<String>,
    /// Detected frameworks
    pub frameworks: Vec<String>,
    /// Module boundaries (top-level directories with code)
    pub boundaries: Vec<String>,
    /// Entry points (main functions, route handlers)
    pub entry_points: Vec<String>,
    /// Module boundaries from Leiden community detection
    pub community_boundaries: Vec<CommunityBoundary>,
    /// Top complexity hotspots
    pub hotspots: Vec<ComplexityHotspot>,
    /// Active ADRs (status "accepted")
    pub active_adrs: Vec<AdrSummary>,
}

/// A module boundary derived from Leiden community detection.
#[derive(Debug, Clone, Serialize)]
pub struct CommunityBoundary {
    /// Community identifier
    pub community_id: usize,
    /// Primary files in this community
    pub files: Vec<String>,
    /// Number of nodes in this community
    pub node_count: usize,
}

/// A complexity hotspot entry.
#[derive(Debug, Clone, Serialize)]
pub struct ComplexityHotspot {
    /// File path
    pub file: String,
    /// Start line
    pub line: u32,
    /// Function name (FQN)
    pub function: String,
    /// Cyclomatic complexity score
    pub complexity: u32,
}

/// Summary of an active ADR for steering output.
#[derive(Debug, Clone, Serialize)]
pub struct AdrSummary {
    /// ADR title
    pub title: String,
    /// One-line summary (first sentence of body)
    pub summary: String,
}

/// Analyze the graph and generate steering file content.
///
/// Returns content as JSON-serializable struct. Does NOT write files to disk.
/// The MCP tool `generate_steering` returns this content for the agent to use.
pub fn generate_steering(store: &StoreManager) -> Result<SteeringContent, AgentError> {
    let conn = store.read_conn();

    // Analyze languages from file extensions in the graph
    let languages = detect_languages(&conn);

    // Analyze frameworks from import patterns
    let frameworks = detect_frameworks(&conn);

    // Detect module boundaries (top-level directories)
    let boundaries = detect_boundaries(&conn);

    // Detect entry points (main functions, route handlers)
    let entry_points = detect_entry_points(&conn);

    // Detect community boundaries from Leiden algorithm
    let community_boundaries = detect_community_boundaries(&conn);

    // Detect complexity hotspots (top 10)
    let hotspots = detect_complexity_hotspots(&conn, 10);

    // Detect active ADRs
    let active_adrs = detect_active_adrs(&conn);

    // Generate content from templates (includes new sections)
    let claude_md = generate_claude_md(
        &languages,
        &frameworks,
        &boundaries,
        &entry_points,
        &community_boundaries,
        &hotspots,
        &active_adrs,
    );
    let agents_md = generate_agents_md(
        &languages,
        &frameworks,
        &boundaries,
        &entry_points,
        &community_boundaries,
        &hotspots,
        &active_adrs,
    );
    let cursorrules = generate_cursorrules(&languages, &frameworks, &boundaries);

    Ok(SteeringContent {
        claude_md,
        agents_md,
        cursorrules,
        languages,
        frameworks,
        boundaries,
        entry_points,
        community_boundaries,
        hotspots,
        active_adrs,
    })
}

/// Detect module boundaries from Leiden community detection.
///
/// Runs community detection with default coupling threshold and returns
/// a summary of each community with its primary files.
fn detect_community_boundaries(conn: &rusqlite::Connection) -> Vec<CommunityBoundary> {
    let result = community::detect_communities(conn, None, 0.5);
    match result {
        Ok(detection) => detection
            .communities
            .into_iter()
            .map(|c| CommunityBoundary {
                community_id: c.community_id,
                files: c.files,
                node_count: c.node_count,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Detect top complexity hotspots from the nodes table.
///
/// Queries Function nodes ordered by cyclomatic complexity descending,
/// returning at most `limit` results.
fn detect_complexity_hotspots(conn: &rusqlite::Connection, limit: usize) -> Vec<ComplexityHotspot> {
    let result = conn.prepare(
        "SELECT fqn, file, start_line, attributes \
         FROM nodes \
         WHERE kind IN ('Function', 'Method') \
           AND CAST(json_extract(attributes, '$.complexity') AS INTEGER) > 0 \
         ORDER BY CAST(json_extract(attributes, '$.complexity') AS INTEGER) DESC \
         LIMIT ?1",
    );

    let mut hotspots = Vec::new();
    if let Ok(mut stmt) = result {
        let rows = stmt.query_map(rusqlite::params![limit], |row| {
            let fqn: String = row.get(0)?;
            let file: String = row.get(1)?;
            let start_line: u32 = row.get(2)?;
            let attrs_str: String = row.get(3)?;
            Ok((fqn, file, start_line, attrs_str))
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (fqn, file, start_line, attrs_str) = row;
                let complexity = serde_json::from_str::<serde_json::Value>(&attrs_str)
                    .ok()
                    .and_then(|v| v.get("complexity")?.as_u64())
                    .unwrap_or(0) as u32;

                hotspots.push(ComplexityHotspot {
                    file,
                    line: start_line,
                    function: fqn,
                    complexity,
                });
            }
        }
    }

    hotspots
}

/// Detect active ADRs (status "accepted") from the store.
///
/// Returns a summary with title and first sentence of the body.
fn detect_active_adrs(conn: &rusqlite::Connection) -> Vec<AdrSummary> {
    let result = memory::read_adrs(conn, None, Some("accepted"));
    match result {
        Ok(adrs) => adrs
            .into_iter()
            .map(|adr| {
                let summary = extract_first_sentence(&adr.body);
                AdrSummary {
                    title: adr.title,
                    summary,
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Extract the first sentence from a body of text.
/// Returns up to the first period followed by a space, or the first 120 chars.
fn extract_first_sentence(body: &str) -> String {
    let trimmed = body.trim();
    if let Some(pos) = trimmed.find(". ") {
        trimmed[..=pos].to_string()
    } else if let Some(pos) = trimmed.find(".\n") {
        trimmed[..=pos].to_string()
    } else if trimmed.len() <= 120 {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..117])
    }
}

/// Token budget constant: maximum estimated tokens for steering content.
const TOKEN_BUDGET: usize = 2000;

/// Estimate token count as chars / 4.
fn estimate_tokens(content: &str) -> usize {
    content.len() / 4
}

/// Apply token budget enforcement to steering content.
///
/// If the content exceeds TOKEN_BUDGET tokens (chars/4), truncates:
/// - Hotspots to top 5
/// - ADR summaries to title-only (no body summary)
fn apply_token_budget(
    content: &str,
    hotspots: &[ComplexityHotspot],
    adrs: &[AdrSummary],
) -> String {
    if estimate_tokens(content) <= TOKEN_BUDGET {
        return content.to_string();
    }

    // Regenerate the hotspots and ADR sections with truncation
    let truncated_hotspots: Vec<&ComplexityHotspot> = hotspots.iter().take(5).collect();
    let truncated_adrs: Vec<&AdrSummary> = adrs.iter().collect();

    // Find and replace the hotspots section
    let mut result = content.to_string();

    // Replace hotspots section with truncated version
    if let Some(start) = result.find("## Complexity Hotspots\n") {
        if let Some(next_section) = result[start + 23..].find("\n## ") {
            let end = start + 23 + next_section;
            let mut new_section = String::from("## Complexity Hotspots\n\n");
            for h in &truncated_hotspots {
                new_section.push_str(&format!(
                    "- `{}:{}` {} (complexity: {})\n",
                    h.file, h.line, h.function, h.complexity
                ));
            }
            result = format!("{}{}{}", &result[..start], new_section, &result[end..]);
        } else {
            // Hotspots is the last section
            let mut new_section = String::from("## Complexity Hotspots\n\n");
            for h in &truncated_hotspots {
                new_section.push_str(&format!(
                    "- `{}:{}` {} (complexity: {})\n",
                    h.file, h.line, h.function, h.complexity
                ));
            }
            result = format!("{}{}\n", &result[..start], new_section);
        }
    }

    // Replace ADR section with title-only version
    if let Some(start) = result.find("## Architectural Decisions\n") {
        if let Some(next_section) = result[start + 27..].find("\n## ") {
            let end = start + 27 + next_section;
            let mut new_section = String::from("## Architectural Decisions\n\n");
            for adr in &truncated_adrs {
                new_section.push_str(&format!("- {}\n", adr.title));
            }
            result = format!("{}{}{}", &result[..start], new_section, &result[end..]);
        } else {
            // ADRs is the last section
            let mut new_section = String::from("## Architectural Decisions\n\n");
            for adr in &truncated_adrs {
                new_section.push_str(&format!("- {}\n", adr.title));
            }
            result = format!("{}{}\n", &result[..start], new_section);
        }
    }

    result
}

/// Detect languages from file extensions in the nodes table.
fn detect_languages(conn: &rusqlite::Connection) -> Vec<String> {
    let mut languages = Vec::new();
    let mut lang_counts: HashMap<String, usize> = HashMap::new();

    let result = conn.prepare("SELECT DISTINCT file FROM nodes WHERE file != ''");

    if let Ok(mut stmt) = result {
        let rows = stmt.query_map([], |row| {
            let file: String = row.get(0)?;
            Ok(file)
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                if let Some(ext) = std::path::Path::new(&row)
                    .extension()
                    .and_then(|e| e.to_str())
                {
                    let lang = extension_to_language(ext);
                    *lang_counts.entry(lang).or_insert(0) += 1;
                }
            }
        }
    }

    // Sort by count descending
    let mut sorted: Vec<_> = lang_counts.into_iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (lang, _) in sorted {
        languages.push(lang);
    }

    languages
}

/// Detect frameworks from import edges and node attributes.
fn detect_frameworks(conn: &rusqlite::Connection) -> Vec<String> {
    let mut frameworks = Vec::new();

    // Check for known framework imports in edges
    let framework_patterns = [
        ("fastapi", "FastAPI"),
        ("flask", "Flask"),
        ("django", "Django"),
        ("express", "Express"),
        ("react", "React"),
        ("next", "Next.js"),
        ("gin-gonic", "Gin"),
        ("echo", "Echo"),
        ("spring", "Spring"),
        ("rails", "Rails"),
        ("actix", "Actix"),
        ("axum", "Axum"),
        ("tokio", "Tokio"),
    ];

    let result = conn.prepare("SELECT target_fqn FROM edges WHERE kind = 'Imports' LIMIT 10000");

    if let Ok(mut stmt) = result {
        let rows = stmt.query_map([], |row| {
            let target: String = row.get(0)?;
            Ok(target)
        });

        if let Ok(rows) = rows {
            let targets: Vec<String> = rows.flatten().collect();
            for (pattern, name) in &framework_patterns {
                if targets.iter().any(|t| t.to_lowercase().contains(pattern)) {
                    frameworks.push(name.to_string());
                }
            }
        }
    }

    // Also check Route nodes for framework attributes
    let route_result = conn.prepare("SELECT attributes FROM nodes WHERE kind = 'Route' LIMIT 100");

    if let Ok(mut stmt) = route_result {
        let rows = stmt.query_map([], |row| {
            let attrs: String = row.get(0)?;
            Ok(attrs)
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                if let Ok(attrs) = serde_json::from_str::<serde_json::Value>(&row)
                    && let Some(fw) = attrs.get("framework").and_then(|v| v.as_str())
                {
                    let fw_name = fw.to_string();
                    if !frameworks.contains(&fw_name) {
                        frameworks.push(fw_name);
                    }
                }
            }
        }
    }

    frameworks
}

/// Detect module boundaries from top-level directories in file paths.
fn detect_boundaries(conn: &rusqlite::Connection) -> Vec<String> {
    let mut boundaries = Vec::new();

    let result = conn.prepare("SELECT DISTINCT file FROM nodes WHERE file != '' LIMIT 5000");

    if let Ok(mut stmt) = result {
        let rows = stmt.query_map([], |row| {
            let file: String = row.get(0)?;
            Ok(file)
        });

        if let Ok(rows) = rows {
            let mut seen = std::collections::HashSet::new();
            for row in rows.flatten() {
                // Extract top-level directory
                if let Some(first_component) = row.split('/').next()
                    && first_component != row
                    && seen.insert(first_component.to_string())
                {
                    boundaries.push(first_component.to_string());
                }
            }
        }
    }

    boundaries.sort();
    boundaries
}

/// Detect entry points (main functions, route handlers).
fn detect_entry_points(conn: &rusqlite::Connection) -> Vec<String> {
    let mut entry_points = Vec::new();

    // Find main functions
    let result = conn.prepare(
        "SELECT fqn FROM nodes WHERE (fqn LIKE '%::main' OR fqn LIKE '%::Main') AND kind = 'Function' LIMIT 10"
    );

    if let Ok(mut stmt) = result {
        let rows = stmt.query_map([], |row| {
            let fqn: String = row.get(0)?;
            Ok(fqn)
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                entry_points.push(row);
            }
        }
    }

    // Find route handlers (limit to first 20)
    let route_result = conn.prepare("SELECT fqn FROM nodes WHERE kind = 'Route' LIMIT 20");

    if let Ok(mut stmt) = route_result {
        let rows = stmt.query_map([], |row| {
            let fqn: String = row.get(0)?;
            Ok(fqn)
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                entry_points.push(row);
            }
        }
    }

    entry_points
}

/// Map file extension to language name.
fn extension_to_language(ext: &str) -> String {
    match ext {
        "py" | "pyi" => "Python".to_string(),
        "ts" | "tsx" => "TypeScript".to_string(),
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript".to_string(),
        "go" => "Go".to_string(),
        "rs" => "Rust".to_string(),
        "java" => "Java".to_string(),
        "cs" => "C#".to_string(),
        "cpp" | "cxx" | "cc" | "hpp" => "C++".to_string(),
        "c" | "h" => "C".to_string(),
        "rb" => "Ruby".to_string(),
        "scala" => "Scala".to_string(),
        "swift" => "Swift".to_string(),
        "php" => "PHP".to_string(),
        "sql" => "SQL".to_string(),
        _ => ext.to_string(),
    }
}

/// Generate CLAUDE.md content from template.
fn generate_claude_md(
    languages: &[String],
    frameworks: &[String],
    boundaries: &[String],
    entry_points: &[String],
    community_boundaries: &[CommunityBoundary],
    hotspots: &[ComplexityHotspot],
    active_adrs: &[AdrSummary],
) -> String {
    let mut content = String::new();
    content.push_str("# Project Context\n\n");

    if !languages.is_empty() {
        content.push_str("## Languages\n\n");
        for lang in languages {
            content.push_str(&format!("- {}\n", lang));
        }
        content.push('\n');
    }

    if !frameworks.is_empty() {
        content.push_str("## Frameworks\n\n");
        for fw in frameworks {
            content.push_str(&format!("- {}\n", fw));
        }
        content.push('\n');
    }

    if !boundaries.is_empty() {
        content.push_str("## Module Boundaries\n\n");
        for boundary in boundaries {
            content.push_str(&format!("- `{}/`\n", boundary));
        }
        content.push('\n');
    }

    if !community_boundaries.is_empty() {
        content.push_str("## Detected Module Clusters\n\n");
        for cb in community_boundaries {
            let primary_files: Vec<&str> = cb.files.iter().take(3).map(|s| s.as_str()).collect();
            content.push_str(&format!(
                "- Cluster {} ({} nodes): {}\n",
                cb.community_id,
                cb.node_count,
                primary_files.join(", ")
            ));
        }
        content.push('\n');
    }

    if !entry_points.is_empty() {
        content.push_str("## Entry Points\n\n");
        for ep in entry_points.iter().take(10) {
            content.push_str(&format!("- `{}`\n", ep));
        }
        content.push('\n');
    }

    if !hotspots.is_empty() {
        content.push_str("## Complexity Hotspots\n\n");
        for h in hotspots {
            content.push_str(&format!(
                "- `{}:{}` {} (complexity: {})\n",
                h.file, h.line, h.function, h.complexity
            ));
        }
        content.push('\n');
    }

    if !active_adrs.is_empty() {
        content.push_str("## Architectural Decisions\n\n");
        for adr in active_adrs {
            content.push_str(&format!("- **{}**: {}\n", adr.title, adr.summary));
        }
        content.push('\n');
    }

    content.push_str("## Guidelines\n\n");
    content.push_str("- Follow existing code conventions and patterns\n");
    content.push_str("- Use the project's established dependency versions\n");
    content.push_str("- Maintain module boundary separation\n");

    // Apply token budget enforcement
    apply_token_budget(&content, hotspots, active_adrs)
}

/// Generate AGENTS.md content from template.
fn generate_agents_md(
    languages: &[String],
    frameworks: &[String],
    boundaries: &[String],
    entry_points: &[String],
    community_boundaries: &[CommunityBoundary],
    hotspots: &[ComplexityHotspot],
    active_adrs: &[AdrSummary],
) -> String {
    let mut content = String::new();
    content.push_str("# Agent Guidelines\n\n");
    content.push_str("## Project Structure\n\n");

    if !languages.is_empty() {
        content.push_str(&format!("Primary languages: {}\n\n", languages.join(", ")));
    }

    if !frameworks.is_empty() {
        content.push_str(&format!("Frameworks: {}\n\n", frameworks.join(", ")));
    }

    if !boundaries.is_empty() {
        content.push_str("### Modules\n\n");
        for boundary in boundaries {
            content.push_str(&format!("- `{}/`\n", boundary));
        }
        content.push('\n');
    }

    if !community_boundaries.is_empty() {
        content.push_str("### Detected Module Clusters\n\n");
        for cb in community_boundaries {
            let primary_files: Vec<&str> = cb.files.iter().take(3).map(|s| s.as_str()).collect();
            content.push_str(&format!(
                "- Cluster {} ({} nodes): {}\n",
                cb.community_id,
                cb.node_count,
                primary_files.join(", ")
            ));
        }
        content.push('\n');
    }

    if !entry_points.is_empty() {
        content.push_str("### Entry Points\n\n");
        for ep in entry_points.iter().take(10) {
            content.push_str(&format!("- `{}`\n", ep));
        }
        content.push('\n');
    }

    if !hotspots.is_empty() {
        content.push_str("## Complexity Hotspots\n\n");
        for h in hotspots {
            content.push_str(&format!(
                "- `{}:{}` {} (complexity: {})\n",
                h.file, h.line, h.function, h.complexity
            ));
        }
        content.push('\n');
    }

    if !active_adrs.is_empty() {
        content.push_str("## Architectural Decisions\n\n");
        for adr in active_adrs {
            content.push_str(&format!("- **{}**: {}\n", adr.title, adr.summary));
        }
        content.push('\n');
    }

    content.push_str("## Conventions\n\n");
    content.push_str("- Respect module boundaries when making changes\n");
    content.push_str("- Follow existing naming conventions\n");
    content.push_str("- Add tests for new functionality\n");

    // Apply token budget enforcement
    apply_token_budget(&content, hotspots, active_adrs)
}

/// Generate .cursorrules content from template.
fn generate_cursorrules(
    languages: &[String],
    frameworks: &[String],
    boundaries: &[String],
) -> String {
    let mut content = String::new();

    if !languages.is_empty() {
        content.push_str(&format!("This project uses: {}\n\n", languages.join(", ")));
    }

    if !frameworks.is_empty() {
        content.push_str(&format!("Frameworks: {}\n\n", frameworks.join(", ")));
    }

    if !boundaries.is_empty() {
        content.push_str("Module structure:\n");
        for boundary in boundaries {
            content.push_str(&format!("- {}/\n", boundary));
        }
        content.push('\n');
    }

    content.push_str("Rules:\n");
    content.push_str("- Follow existing code patterns and conventions\n");
    content.push_str("- Maintain module boundary separation\n");
    content.push_str("- Use established dependency versions\n");
    content.push_str("- Add tests for new functionality\n");

    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::StoreManager;
    use crate::store::migrations;
    use tempfile::TempDir;

    fn setup_store() -> (StoreManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = StoreManager::new(tmp.path()).unwrap();
        let conn = store.write_conn();
        migrations::run_migrations(&conn, std::path::Path::new("migrations")).unwrap();
        drop(conn);
        (store, tmp)
    }

    #[test]
    fn test_correct_language_framework_detection() {
        let (store, _tmp) = setup_store();

        // Insert nodes with different file extensions
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.py::main', 'Function', 'src/main.py', 1, 10, 'h1', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/app.ts::handler', 'Function', 'src/app.ts', 1, 10, 'h2', 1000, '{}')",
                [],
            ).unwrap();
            // Insert a route node with framework attribute
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/routes.py::route::GET:/api', 'Route', 'src/routes.py', 5, 5, 'h3', 1000, '{\"framework\":\"fastapi\",\"method\":\"GET\",\"path\":\"/api\"}')",
                [],
            ).unwrap();
        }

        let content = generate_steering(&store).unwrap();

        // Should detect Python and TypeScript
        assert!(content.languages.contains(&"Python".to_string()));
        assert!(content.languages.contains(&"TypeScript".to_string()));

        // Should detect fastapi framework from route node
        assert!(content.frameworks.contains(&"fastapi".to_string()));
    }

    #[test]
    fn test_content_contains_no_ai_claims() {
        let (store, _tmp) = setup_store();

        let content = generate_steering(&store).unwrap();

        // Verify no AI-generated claims in the content
        let all_content = format!(
            "{}\n{}\n{}",
            content.claude_md, content.agents_md, content.cursorrules
        );

        // Should not contain AI-generated marketing claims
        assert!(!all_content.contains("AI-powered"));
        assert!(!all_content.contains("intelligent"));
        assert!(!all_content.contains("smart"));
        assert!(!all_content.contains("machine learning"));
        assert!(!all_content.contains("neural"));

        // Should contain factual template content
        assert!(all_content.contains("conventions"));
        assert!(all_content.contains("module"));
    }

    // -----------------------------------------------------------------------
    // Tests for install-time steering file functions (Requirement 8)
    // -----------------------------------------------------------------------

    #[test]
    fn test_steering_file_path_cursor() {
        let root = Path::new("/repo");
        let path = steering_file_path("cursor", root);
        assert_eq!(path, root.join(".cursor").join("rules").join("cortex.mdc"));
    }

    #[test]
    fn test_steering_file_path_cursor_case_insensitive() {
        let root = Path::new("/repo");
        let path = steering_file_path("Cursor", root);
        assert_eq!(path, root.join(".cursor").join("rules").join("cortex.mdc"));
    }

    #[test]
    fn test_steering_file_path_claude_code() {
        let root = Path::new("/repo");
        let path = steering_file_path("claude code", root);
        assert_eq!(path, root.join(".claude").join("CLAUDE.md"));
    }

    #[test]
    fn test_steering_file_path_claude_shorthand() {
        let root = Path::new("/repo");
        let path = steering_file_path("claude", root);
        assert_eq!(path, root.join(".claude").join("CLAUDE.md"));
    }

    #[test]
    fn test_steering_file_path_kiro() {
        let root = Path::new("/repo");
        let path = steering_file_path("kiro", root);
        assert_eq!(path, root.join(".kiro").join("steering").join("cortex.md"));
    }

    #[test]
    fn test_steering_file_path_windsurf() {
        let root = Path::new("/repo");
        let path = steering_file_path("windsurf", root);
        assert_eq!(path, root.join(".windsurfrules"));
    }

    #[test]
    fn test_steering_file_path_copilot() {
        let root = Path::new("/repo");
        let path = steering_file_path("copilot", root);
        assert_eq!(path, root.join(".github").join("copilot-instructions.md"));
    }

    #[test]
    fn test_steering_file_path_fallback() {
        let root = Path::new("/repo");
        let path = steering_file_path("unknown-agent", root);
        assert_eq!(path, root.join(".cortex").join("steering.md"));
    }

    #[test]
    fn test_contains_cortex_content_with_markers() {
        let content = "Some existing content\n<!-- cortex-steering-start -->\nCortex stuff\n<!-- cortex-steering-end -->\nMore content";
        assert!(contains_cortex_content(content));
    }

    #[test]
    fn test_contains_cortex_content_without_markers() {
        let content = "Some existing content\nNo cortex markers here\n";
        assert!(!contains_cortex_content(content));
    }

    #[test]
    fn test_contains_cortex_content_only_start_marker() {
        let content = "<!-- cortex-steering-start -->\nContent without end marker";
        assert!(!contains_cortex_content(content));
    }

    #[test]
    fn test_contains_cortex_content_only_end_marker() {
        let content = "Content without start marker\n<!-- cortex-steering-end -->";
        assert!(!contains_cortex_content(content));
    }

    #[test]
    fn test_write_steering_file_creates_new_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_steering_file("cursor", root, STEERING_TEMPLATE).unwrap();

        let path = root.join(".cursor").join("rules").join("cortex.mdc");
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(contains_cortex_content(&content));
        assert!(content.contains("Cortex MCP Tools"));
        assert!(content.contains("ask"));
        assert!(content.contains("search_symbols"));
    }

    #[test]
    fn test_write_steering_file_creates_directories() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Kiro path requires nested directories
        write_steering_file("kiro", root, STEERING_TEMPLATE).unwrap();

        let path = root.join(".kiro").join("steering").join("cortex.md");
        assert!(path.exists());
    }

    #[test]
    fn test_write_steering_file_appends_to_existing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create an existing file without cortex content
        let path = root.join(".windsurfrules");
        std::fs::write(&path, "# Existing rules\n\nDo not delete files.\n").unwrap();

        write_steering_file("windsurf", root, STEERING_TEMPLATE).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        // Should contain both original and cortex content
        assert!(content.contains("Existing rules"));
        assert!(content.contains("Do not delete files."));
        assert!(contains_cortex_content(&content));
    }

    #[test]
    fn test_write_steering_file_replaces_existing_cortex_section() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let path = root.join(".windsurfrules");
        let existing = "# Rules\n\n<!-- cortex-steering-start -->\nOld cortex content\n<!-- cortex-steering-end -->\n\n# More rules\n";
        std::fs::write(&path, existing).unwrap();

        write_steering_file("windsurf", root, STEERING_TEMPLATE).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        // Should NOT contain old content
        assert!(!content.contains("Old cortex content"));
        // Should contain new template content
        assert!(content.contains("Cortex MCP Tools"));
        // Should preserve surrounding content
        assert!(content.contains("# Rules"));
        assert!(content.contains("# More rules"));
        // Should not duplicate markers
        assert_eq!(
            content.matches(CORTEX_STEERING_START).count(),
            1,
            "Start marker should appear exactly once"
        );
        assert_eq!(
            content.matches(CORTEX_STEERING_END).count(),
            1,
            "End marker should appear exactly once"
        );
    }

    #[test]
    fn test_write_steering_file_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Write twice
        write_steering_file("cursor", root, STEERING_TEMPLATE).unwrap();
        let first_content = std::fs::read_to_string(steering_file_path("cursor", root)).unwrap();

        write_steering_file("cursor", root, STEERING_TEMPLATE).unwrap();
        let second_content = std::fs::read_to_string(steering_file_path("cursor", root)).unwrap();

        assert_eq!(
            first_content, second_content,
            "Writing twice should produce identical content"
        );
    }

    #[test]
    fn test_write_steering_file_with_empty_content_uses_template() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_steering_file("cursor", root, "").unwrap();

        let path = steering_file_path("cursor", root);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(contains_cortex_content(&content));
        assert!(content.contains("Cortex MCP Tools"));
    }

    #[test]
    fn test_steering_template_contains_required_tools() {
        // Verify the template mentions all required tools per Requirement 8.7
        assert!(STEERING_TEMPLATE.contains("ask"));
        assert!(STEERING_TEMPLATE.contains("search_symbols"));
        assert!(STEERING_TEMPLATE.contains("trace_callers"));
        assert!(STEERING_TEMPLATE.contains("get_file_context"));
        assert!(STEERING_TEMPLATE.contains("blast_radius"));
    }

    #[test]
    fn test_steering_template_has_markers() {
        assert!(STEERING_TEMPLATE.starts_with(CORTEX_STEERING_START));
        assert!(STEERING_TEMPLATE.ends_with(CORTEX_STEERING_END));
    }

    #[test]
    fn test_replace_cortex_section_preserves_surrounding() {
        let existing =
            "Before\n<!-- cortex-steering-start -->\nOld\n<!-- cortex-steering-end -->\nAfter\n";
        let new_section = "<!-- cortex-steering-start -->\nNew\n<!-- cortex-steering-end -->";
        let result = replace_cortex_section(existing, new_section);
        assert!(result.contains("Before"));
        assert!(result.contains("After"));
        assert!(result.contains("New"));
        assert!(!result.contains("Old"));
    }

    // ─── Tests for steering improvements (Requirement 10) ─────────────────────

    #[test]
    fn test_extract_first_sentence_with_period_space() {
        let body = "Use event sourcing for state. This allows replay and audit.";
        assert_eq!(
            extract_first_sentence(body),
            "Use event sourcing for state."
        );
    }

    #[test]
    fn test_extract_first_sentence_with_period_newline() {
        let body = "Use event sourcing for state.\nThis allows replay and audit.";
        assert_eq!(
            extract_first_sentence(body),
            "Use event sourcing for state."
        );
    }

    #[test]
    fn test_extract_first_sentence_short_body() {
        let body = "Short body without period";
        assert_eq!(extract_first_sentence(body), "Short body without period");
    }

    #[test]
    fn test_extract_first_sentence_long_body_no_period() {
        let body = "A".repeat(200);
        let result = extract_first_sentence(&body);
        assert_eq!(result.len(), 120); // 117 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        // 8000 chars = 2000 tokens
        let content = "a".repeat(8000);
        assert_eq!(estimate_tokens(&content), 2000);
    }

    #[test]
    fn test_token_budget_under_limit_returns_unchanged() {
        let content = "Short content";
        let hotspots = vec![];
        let adrs = vec![];
        let result = apply_token_budget(content, &hotspots, &adrs);
        assert_eq!(result, content);
    }

    #[test]
    fn test_token_budget_over_limit_truncates_hotspots() {
        // Create content that exceeds 2000 tokens (8000 chars)
        let mut content = String::from("# Header\n\n");
        content.push_str("## Complexity Hotspots\n\n");
        let mut hotspots = Vec::new();
        for i in 0..10 {
            let h = ComplexityHotspot {
                file: format!("src/very/long/path/to/module_{}.rs", i),
                line: i * 10 + 1,
                function: format!(
                    "src/very/long/path/to/module_{}.rs::very_complex_function_name_{}",
                    i, i
                ),
                complexity: 20 - i,
            };
            content.push_str(&format!(
                "- `{}:{}` {} (complexity: {})\n",
                h.file, h.line, h.function, h.complexity
            ));
            hotspots.push(h);
        }
        content.push('\n');
        content.push_str("## Guidelines\n\n");
        // Pad to exceed budget
        content.push_str(&"x".repeat(8000));

        let result = apply_token_budget(&content, &hotspots, &[]);
        // Should still contain the first 5 hotspots
        assert!(result.contains("module_0.rs"));
        assert!(result.contains("module_4.rs"));
        // Should NOT contain hotspots 5-9 (they were truncated)
        assert!(!result.contains("module_5.rs"));
        assert!(!result.contains("module_9.rs"));
    }

    #[test]
    fn test_token_budget_over_limit_truncates_adrs_to_title_only() {
        let mut content = String::from("# Header\n\n");
        content.push_str("## Architectural Decisions\n\n");
        let adrs = vec![
            AdrSummary {
                title: "Use Event Sourcing".to_string(),
                summary: "Event sourcing provides full audit trail.".to_string(),
            },
            AdrSummary {
                title: "Adopt Microservices".to_string(),
                summary: "Microservices enable independent deployment.".to_string(),
            },
        ];
        for adr in &adrs {
            content.push_str(&format!("- **{}**: {}\n", adr.title, adr.summary));
        }
        content.push('\n');
        content.push_str("## Guidelines\n\n");
        // Pad to exceed budget
        content.push_str(&"x".repeat(8000));

        let result = apply_token_budget(&content, &[], &adrs);
        // Should contain titles
        assert!(result.contains("Use Event Sourcing"));
        assert!(result.contains("Adopt Microservices"));
        // Should NOT contain summaries (title-only mode)
        assert!(!result.contains("Event sourcing provides"));
        assert!(!result.contains("Microservices enable"));
    }

    #[test]
    fn test_generate_steering_includes_community_boundaries() {
        let (store, _tmp) = setup_store();

        // Insert nodes that form a community
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/auth/login.rs::login', 'Function', 'src/auth/login.rs', 1, 10, 'h1', 1000, '{\"complexity\": 5}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/auth/verify.rs::verify', 'Function', 'src/auth/verify.rs', 1, 10, 'h2', 1000, '{\"complexity\": 3}')",
                [],
            ).unwrap();
            // Add an edge between them to form a community
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence) \
                 VALUES ('src/auth/login.rs::login', 'src/auth/verify.rs::verify', 'Calls', 1.0)",
                [],
            )
            .unwrap();
        }

        let content = generate_steering(&store).unwrap();

        // Should have detected hotspots
        assert!(!content.hotspots.is_empty());
        assert_eq!(content.hotspots[0].complexity, 5);
        assert_eq!(content.hotspots[0].function, "src/auth/login.rs::login");
    }

    #[test]
    fn test_generate_steering_includes_active_adrs() {
        let (store, _tmp) = setup_store();

        // Insert ADRs with different statuses
        {
            let conn = store.write_conn();
            memory::write_adr(
                &conn,
                "Use Event Sourcing",
                "Event sourcing provides full audit trail. It enables replay.",
                "accepted",
                None,
            )
            .unwrap();
            memory::write_adr(
                &conn,
                "Consider GraphQL",
                "GraphQL might simplify our API. Needs evaluation.",
                "proposed",
                None,
            )
            .unwrap();
            memory::write_adr(
                &conn,
                "Old Decision",
                "This was deprecated.",
                "deprecated",
                None,
            )
            .unwrap();
        }

        let content = generate_steering(&store).unwrap();

        // Should include only accepted ADRs
        assert_eq!(content.active_adrs.len(), 1);
        assert_eq!(content.active_adrs[0].title, "Use Event Sourcing");
        assert_eq!(
            content.active_adrs[0].summary,
            "Event sourcing provides full audit trail."
        );

        // Should be in the generated content
        assert!(content.claude_md.contains("Use Event Sourcing"));
        assert!(!content.claude_md.contains("Consider GraphQL"));
        assert!(!content.claude_md.contains("Old Decision"));
    }

    #[test]
    fn test_generate_steering_includes_complexity_hotspots() {
        let (store, _tmp) = setup_store();

        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/parser.rs::parse', 'Function', 'src/parser.rs', 10, 50, 'h1', 1000, '{\"complexity\": 15}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/simple.rs::hello', 'Function', 'src/simple.rs', 1, 3, 'h2', 1000, '{\"complexity\": 1}')",
                [],
            ).unwrap();
        }

        let content = generate_steering(&store).unwrap();

        // Should include the complex function
        assert_eq!(content.hotspots.len(), 2);
        assert_eq!(content.hotspots[0].function, "src/parser.rs::parse");
        assert_eq!(content.hotspots[0].complexity, 15);

        // Should be in the generated content
        assert!(content.claude_md.contains("src/parser.rs:10"));
        assert!(content.claude_md.contains("complexity: 15"));
    }

    // ─── Property Tests ───────────────────────────────────────────────────────

    use proptest::prelude::*;

    /// Strategy to generate valid agent names (known agents + arbitrary fallback).
    fn agent_name_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("cursor".to_string()),
            Just("claude code".to_string()),
            Just("claude".to_string()),
            Just("kiro".to_string()),
            Just("windsurf".to_string()),
            Just("copilot".to_string()),
            "[a-z][a-z0-9 _-]{0,20}".prop_map(|s| s),
        ]
    }

    /// Strategy to generate arbitrary initial file content that may or may not
    /// already contain cortex steering markers.
    fn initial_content_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            // Empty file (no pre-existing content)
            Just(String::new()),
            // File with some content but no cortex markers
            "[a-zA-Z0-9 \n#_\\-]{0,200}".prop_map(|s| s),
            // File that already has cortex markers (simulates prior install)
            "[a-zA-Z0-9 \n#_\\-]{0,50}".prop_map(|prefix| {
                format!(
                    "{}<!-- cortex-steering-start -->\nOld content\n<!-- cortex-steering-end -->\n",
                    if prefix.is_empty() {
                        String::new()
                    } else {
                        format!("{}\n", prefix)
                    }
                )
            }),
        ]
    }

    // **Property 14: Steering file write is idempotent**
    //
    // For any agent name and existing file content, writing the steering file
    // twice in succession SHALL produce the same file content as writing it once.
    // The Cortex steering section SHALL not be duplicated.
    //
    // **Validates: Requirements 8.8**

    /// Strategy to generate a random ADR status.
    fn adr_status_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("accepted".to_string()),
            Just("proposed".to_string()),
            Just("deprecated".to_string()),
        ]
    }

    /// Strategy to generate a random ADR (title, body, status).
    fn adr_strategy() -> impl Strategy<Value = (String, String, String)> {
        (
            "[A-Za-z ]{1,40}",  // title
            "[A-Za-z .]{1,80}", // body
            adr_status_strategy(),
        )
    }

    proptest! {
        /// Property 10: Steering includes only accepted ADRs
        ///
        /// For any set of ADRs in the store, the generated steering content
        /// SHALL include only those with status "accepted" and SHALL include
        /// none with status "proposed" or "deprecated".
        ///
        /// **Validates: Requirements 10.3**
        #[test]
        fn prop_steering_only_accepted_adrs(
            adrs in proptest::collection::vec(adr_strategy(), 1..20),
        ) {
            let (store, _tmp) = setup_store();

            // Deduplicate by title: if the same title appears with different statuses,
            // only the last one wins (since write_adr inserts a new row each time,
            // but we track by title in this test). Use indexed titles to avoid collisions.
            let indexed_adrs: Vec<(String, String, String)> = adrs
                .iter()
                .enumerate()
                .map(|(i, (title, body, status))| {
                    (format!("ADR-{}: {}", i, title), body.clone(), status.clone())
                })
                .collect();

            // Insert ADRs with various statuses
            {
                let conn = store.write_conn();
                for (title, body, status) in &indexed_adrs {
                    memory::write_adr(&conn, title, body, status, None).unwrap();
                }
            }

            // Call detect_active_adrs
            let conn = store.read_conn();
            let active = detect_active_adrs(&conn);

            // Count expected accepted ADRs
            let expected_count = indexed_adrs.iter().filter(|(_, _, s)| s == "accepted").count();
            prop_assert_eq!(
                active.len(),
                expected_count,
                "Expected {} accepted ADRs, got {}",
                expected_count,
                active.len()
            );

            // Verify all returned ADRs correspond to accepted ones
            let accepted_titles: Vec<&str> = indexed_adrs
                .iter()
                .filter(|(_, _, s)| s == "accepted")
                .map(|(t, _, _)| t.as_str())
                .collect();

            for adr_summary in &active {
                prop_assert!(
                    accepted_titles.contains(&adr_summary.title.as_str()),
                    "ADR '{}' was returned but is not in the accepted set",
                    adr_summary.title
                );
            }

            // Verify no proposed or deprecated ADRs are included
            let non_accepted_titles: Vec<&str> = indexed_adrs
                .iter()
                .filter(|(_, _, s)| s != "accepted")
                .map(|(t, _, _)| t.as_str())
                .collect();

            for adr_summary in &active {
                prop_assert!(
                    !non_accepted_titles.contains(&adr_summary.title.as_str()),
                    "ADR '{}' has non-accepted status but was returned",
                    adr_summary.title
                );
            }
        }

        /// Property 14a: Writing the steering file twice produces the same result
        /// as writing it once. The cortex-steering markers appear exactly once.
        ///
        /// **Validates: Requirements 8.8**
        #[test]
        fn prop_steering_write_idempotent(
            agent_name in agent_name_strategy(),
            initial_content in initial_content_strategy(),
        ) {
            let tmp = TempDir::new().unwrap();
            let root = tmp.path();

            // If there's initial content, pre-populate the steering file
            if !initial_content.is_empty() {
                let path = steering_file_path(&agent_name, root);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&path, &initial_content).unwrap();
            }

            // First write
            write_steering_file(&agent_name, root, STEERING_TEMPLATE).unwrap();
            let path = steering_file_path(&agent_name, root);
            let after_first_write = std::fs::read_to_string(&path).unwrap();

            // Second write (should be idempotent)
            write_steering_file(&agent_name, root, STEERING_TEMPLATE).unwrap();
            let after_second_write = std::fs::read_to_string(&path).unwrap();

            // Assert idempotency: both writes produce identical content
            prop_assert_eq!(
                &after_first_write,
                &after_second_write,
                "Writing steering file twice should produce identical content"
            );

            // Assert no duplication: markers appear exactly once
            let start_count = after_second_write.matches(CORTEX_STEERING_START).count();
            let end_count = after_second_write.matches(CORTEX_STEERING_END).count();
            prop_assert_eq!(
                start_count, 1,
                "Start marker should appear exactly once, found {}",
                start_count
            );
            prop_assert_eq!(
                end_count, 1,
                "End marker should appear exactly once, found {}",
                end_count
            );
        }

        /// Property 11: Steering token budget
        ///
        /// For any project, generated steering content does not exceed 2000 tokens
        /// (chars / 4). After apply_token_budget(), the result is always within budget.
        ///
        /// **Validates: Requirements 10.4**
        #[test]
        fn prop_steering_token_budget(
            num_hotspots in 0usize..50,
            num_adrs in 0usize..40,
            file_path_len in 10usize..80,
            fn_name_len in 5usize..60,
            title_len in 5usize..100,
            summary_len in 10usize..200,
        ) {
            // Generate hotspots with varying content lengths
            let hotspots: Vec<ComplexityHotspot> = (0..num_hotspots)
                .map(|i| ComplexityHotspot {
                    file: format!("src/{}", "m".repeat(file_path_len.min(70))),
                    line: i as u32 * 10 + 1,
                    function: format!(
                        "{}::{}",
                        "m".repeat(file_path_len.min(70)),
                        "f".repeat(fn_name_len.min(50))
                    ),
                    complexity: 30 - (i as u32 % 25),
                })
                .collect();

            // Generate ADRs with varying content lengths
            let adrs: Vec<AdrSummary> = (0..num_adrs)
                .map(|i| AdrSummary {
                    title: format!("ADR-{}: {}", i, "t".repeat(title_len.min(90))),
                    summary: "s".repeat(summary_len.min(190)),
                })
                .collect();

            // Build steering content similar to generate_claude_md
            let mut content = String::from("# Project Context\n\n");
            content.push_str("## Languages\n\n- Rust\n- Python\n- TypeScript\n\n");
            content.push_str("## Frameworks\n\n- Axum\n- Tokio\n\n");

            if !hotspots.is_empty() {
                content.push_str("## Complexity Hotspots\n\n");
                for h in &hotspots {
                    content.push_str(&format!(
                        "- `{}:{}` {} (complexity: {})\n",
                        h.file, h.line, h.function, h.complexity
                    ));
                }
                content.push('\n');
            }

            if !adrs.is_empty() {
                content.push_str("## Architectural Decisions\n\n");
                for adr in &adrs {
                    content.push_str(&format!("- **{}**: {}\n", adr.title, adr.summary));
                }
                content.push('\n');
            }

            content.push_str("## Guidelines\n\n");
            content.push_str("- Follow existing code conventions and patterns\n");
            content.push_str("- Use the project's established dependency versions\n");
            content.push_str("- Maintain module boundary separation\n");

            // Apply token budget enforcement
            let result = apply_token_budget(&content, &hotspots, &adrs);

            // Property: result must not exceed TOKEN_BUDGET tokens
            let tokens = estimate_tokens(&result);
            prop_assert!(
                tokens <= TOKEN_BUDGET,
                "Steering content exceeded token budget: {} tokens (max {}), {} chars",
                tokens,
                TOKEN_BUDGET,
                result.len()
            );
        }
    }
}
