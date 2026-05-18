//! Steering file generation: analyzes the graph to produce agent-specific
//! context files (CLAUDE.md, AGENTS.md, .cursorrules).
//!
//! Content is generated from templates based on detected languages, frameworks,
//! module boundaries, and entry points. No LLM-generated content is used.

use std::collections::HashMap;

use serde::Serialize;

use crate::error::AgentError;
use crate::store::db::StoreManager;

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

    // Generate content from templates
    let claude_md = generate_claude_md(&languages, &frameworks, &boundaries, &entry_points);
    let agents_md = generate_agents_md(&languages, &frameworks, &boundaries, &entry_points);
    let cursorrules = generate_cursorrules(&languages, &frameworks, &boundaries);

    Ok(SteeringContent {
        claude_md,
        agents_md,
        cursorrules,
        languages,
        frameworks,
        boundaries,
        entry_points,
    })
}

/// Detect languages from file extensions in the nodes table.
fn detect_languages(conn: &rusqlite::Connection) -> Vec<String> {
    let mut languages = Vec::new();
    let mut lang_counts: HashMap<String, usize> = HashMap::new();

    let result = conn.prepare(
        "SELECT DISTINCT file FROM nodes WHERE file != ''"
    );

    if let Ok(mut stmt) = result {
        let rows = stmt.query_map([], |row| {
            let file: String = row.get(0)?;
            Ok(file)
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                if let Some(ext) = std::path::Path::new(&row).extension().and_then(|e| e.to_str()) {
                    let lang = extension_to_language(ext);
                    *lang_counts.entry(lang).or_insert(0) += 1;
                }
            }
        }
    }

    // Sort by count descending
    let mut sorted: Vec<_> = lang_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
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

    let result = conn.prepare(
        "SELECT target_fqn FROM edges WHERE kind = 'Imports' LIMIT 10000"
    );

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
    let route_result = conn.prepare(
        "SELECT attributes FROM nodes WHERE kind = 'Route' LIMIT 100"
    );

    if let Ok(mut stmt) = route_result {
        let rows = stmt.query_map([], |row| {
            let attrs: String = row.get(0)?;
            Ok(attrs)
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                if let Ok(attrs) = serde_json::from_str::<serde_json::Value>(&row) {
                    if let Some(fw) = attrs.get("framework").and_then(|v| v.as_str()) {
                        let fw_name = fw.to_string();
                        if !frameworks.contains(&fw_name) {
                            frameworks.push(fw_name);
                        }
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

    let result = conn.prepare(
        "SELECT DISTINCT file FROM nodes WHERE file != '' LIMIT 5000"
    );

    if let Ok(mut stmt) = result {
        let rows = stmt.query_map([], |row| {
            let file: String = row.get(0)?;
            Ok(file)
        });

        if let Ok(rows) = rows {
            let mut seen = std::collections::HashSet::new();
            for row in rows.flatten() {
                // Extract top-level directory
                if let Some(first_component) = row.split('/').next() {
                    if first_component != row && seen.insert(first_component.to_string()) {
                        boundaries.push(first_component.to_string());
                    }
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
    let route_result = conn.prepare(
        "SELECT fqn FROM nodes WHERE kind = 'Route' LIMIT 20"
    );

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

    if !entry_points.is_empty() {
        content.push_str("## Entry Points\n\n");
        for ep in entry_points.iter().take(10) {
            content.push_str(&format!("- `{}`\n", ep));
        }
        content.push('\n');
    }

    content.push_str("## Guidelines\n\n");
    content.push_str("- Follow existing code conventions and patterns\n");
    content.push_str("- Use the project's established dependency versions\n");
    content.push_str("- Maintain module boundary separation\n");

    content
}

/// Generate AGENTS.md content from template.
fn generate_agents_md(
    languages: &[String],
    frameworks: &[String],
    boundaries: &[String],
    entry_points: &[String],
) -> String {
    let mut content = String::new();
    content.push_str("# Agent Guidelines\n\n");
    content.push_str("## Project Structure\n\n");

    if !languages.is_empty() {
        content.push_str(&format!(
            "Primary languages: {}\n\n",
            languages.join(", ")
        ));
    }

    if !frameworks.is_empty() {
        content.push_str(&format!(
            "Frameworks: {}\n\n",
            frameworks.join(", ")
        ));
    }

    if !boundaries.is_empty() {
        content.push_str("### Modules\n\n");
        for boundary in boundaries {
            content.push_str(&format!("- `{}/`\n", boundary));
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

    content.push_str("## Conventions\n\n");
    content.push_str("- Respect module boundaries when making changes\n");
    content.push_str("- Follow existing naming conventions\n");
    content.push_str("- Add tests for new functionality\n");

    content
}

/// Generate .cursorrules content from template.
fn generate_cursorrules(
    languages: &[String],
    frameworks: &[String],
    boundaries: &[String],
) -> String {
    let mut content = String::new();

    if !languages.is_empty() {
        content.push_str(&format!(
            "This project uses: {}\n\n",
            languages.join(", ")
        ));
    }

    if !frameworks.is_empty() {
        content.push_str(&format!(
            "Frameworks: {}\n\n",
            frameworks.join(", ")
        ));
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
}
