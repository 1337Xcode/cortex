//! The `ask` meta-tool: single-call code intelligence.
//!
//! Takes a natural language question and internally routes to the appropriate
//! graph queries, composing a unified answer. The agent never needs to choose
//! between 31 tools. One call, one answer.
//!
//! Routing logic:
//! - "what calls X" / "who calls X" -> trace_callers
//! - "what does X call" -> trace_callees
//! - "what breaks if I change X" / "impact of X" -> blast_radius
//! - "explain X" -> callers + callees + security flags + observations
//! - "find X" / "where is X" -> search_symbols
//! - "security" / "taint" / "vulnerability" -> find_taint_paths + scan_owasp
//! - "dead code" / "unused" -> find_dead_code
//! - "architecture" / "overview" -> get_architecture
//! - fallback -> search_symbols + get_file_context

use serde_json::Value;

use crate::error::McpError;
use crate::store::db::StoreManager;

use super::dispatch::dispatch_tool;

/// Intent detected from the natural language question.
#[derive(Debug, Clone, PartialEq)]
enum Intent {
    TraceCallers,
    TraceCallees,
    BlastRadius,
    Explain,
    Search,
    Security,
    DeadCode,
    Architecture,
    Fallback,
}

/// Dispatch the `ask` meta-tool. Parses the question, determines intent,
/// calls the appropriate internal tools, and composes a unified response.
pub fn dispatch_ask(store: &StoreManager, args: &Value) -> Result<(String, usize), McpError> {
    let question = args
        .get("question")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: question".to_string(),
        })?;

    let lower = question.to_lowercase();
    let intent = classify_intent(&lower);
    let symbol = extract_symbol(question);

    let mut tools_used: Vec<&str> = Vec::new();
    let mut results: Vec<Value> = Vec::new();
    let mut total_files = 0usize;

    match intent {
        Intent::TraceCallers => {
            let fqn = symbol.as_deref().unwrap_or("");
            if fqn.is_empty() {
                return Err(McpError::DispatchError {
                    reason: "could not extract a symbol name from the question. Please include a fully qualified name (FQN).".to_string(),
                });
            }
            let tool_args = serde_json::json!({ "fqn": fqn, "depth": 3 });
            let result = dispatch_tool(store, "trace_callers", &tool_args)?;
            tools_used.push("trace_callers");
            results.push(result);
        }
        Intent::TraceCallees => {
            let fqn = symbol.as_deref().unwrap_or("");
            if fqn.is_empty() {
                return Err(McpError::DispatchError {
                    reason: "could not extract a symbol name from the question. Please include a fully qualified name (FQN).".to_string(),
                });
            }
            let tool_args = serde_json::json!({ "fqn": fqn, "depth": 3 });
            let result = dispatch_tool(store, "trace_callees", &tool_args)?;
            tools_used.push("trace_callees");
            results.push(result);
        }
        Intent::BlastRadius => {
            let fqn = symbol.as_deref().unwrap_or("");
            if fqn.is_empty() {
                return Err(McpError::DispatchError {
                    reason: "could not extract a symbol name from the question. Please include a fully qualified name (FQN).".to_string(),
                });
            }
            let tool_args = serde_json::json!({ "fqn": fqn, "depth": 3 });
            let result = dispatch_tool(store, "blast_radius", &tool_args)?;
            tools_used.push("blast_radius");
            results.push(result);
        }
        Intent::Explain => {
            let fqn = symbol.as_deref().unwrap_or("");
            if fqn.is_empty() {
                return Err(McpError::DispatchError {
                    reason: "could not extract a symbol name from the question. Please include a fully qualified name (FQN).".to_string(),
                });
            }
            // Callers
            let callers_args = serde_json::json!({ "fqn": fqn, "depth": 2 });
            if let Ok(r) = dispatch_tool(store, "trace_callers", &callers_args) {
                tools_used.push("trace_callers");
                results.push(r);
            }
            // Callees
            let callees_args = serde_json::json!({ "fqn": fqn, "depth": 2 });
            if let Ok(r) = dispatch_tool(store, "trace_callees", &callees_args) {
                tools_used.push("trace_callees");
                results.push(r);
            }
            // Observations
            let obs_args = serde_json::json!({ "fqn": fqn, "include_stale": false });
            if let Ok(r) = dispatch_tool(store, "read_observations", &obs_args) {
                tools_used.push("read_observations");
                results.push(r);
            }
        }
        Intent::Search => {
            let pattern = symbol.as_deref().unwrap_or("*");
            let search_pattern = if pattern.contains('*') || pattern.contains("::") {
                pattern.to_string()
            } else {
                format!("*{}*", pattern)
            };
            let tool_args = serde_json::json!({ "pattern": search_pattern, "limit": 20 });
            let result = dispatch_tool(store, "search_symbols", &tool_args)?;
            tools_used.push("search_symbols");
            results.push(result);
        }
        Intent::Security => {
            // Taint paths
            let taint_args = serde_json::json!({});
            if let Ok(r) = dispatch_tool(store, "find_taint_paths", &taint_args) {
                tools_used.push("find_taint_paths");
                results.push(r);
            }
            // OWASP scan
            let owasp_args = serde_json::json!({});
            if let Ok(r) = dispatch_tool(store, "scan_owasp", &owasp_args) {
                tools_used.push("scan_owasp");
                results.push(r);
            }
        }
        Intent::DeadCode => {
            let tool_args = serde_json::json!({ "limit": 30 });
            let result = dispatch_tool(store, "find_dead_code", &tool_args)?;
            tools_used.push("find_dead_code");
            results.push(result);
        }
        Intent::Architecture => {
            let tool_args = serde_json::json!({});
            let result = dispatch_tool(store, "get_architecture", &tool_args)?;
            tools_used.push("get_architecture");
            results.push(result);
        }
        Intent::Fallback => {
            // Try search first
            let pattern = symbol.as_deref().unwrap_or("");
            if !pattern.is_empty() {
                let search_pattern = if pattern.contains('*') || pattern.contains("::") {
                    pattern.to_string()
                } else {
                    format!("*{}*", pattern)
                };
                let tool_args = serde_json::json!({ "pattern": search_pattern, "limit": 20 });
                if let Ok(r) = dispatch_tool(store, "search_symbols", &tool_args) {
                    tools_used.push("search_symbols");
                    results.push(r);
                }
            } else {
                // No symbol found, try text search with the whole question
                let tool_args = serde_json::json!({ "query": question, "limit": 10 });
                if let Ok(r) = dispatch_tool(store, "search_text", &tool_args) {
                    tools_used.push("search_text");
                    results.push(r);
                }
            }
        }
    }

    // Count files from results
    for result in &results {
        if let Some(meta) = result.get("_meta") {
            if let Some(files) = meta.get("files_touched").and_then(|v| v.as_u64()) {
                total_files += files as usize;
            }
        }
    }

    // Compose unified response
    let response = serde_json::json!({
        "question": question,
        "intent": format!("{:?}", intent),
        "symbol": symbol,
        "results": results,
        "_meta": {
            "tools_used": tools_used,
            "tool_count": tools_used.len(),
        }
    });

    let json = serde_json::to_string(&response).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize ask response: {}", e),
    })?;

    Ok((json, total_files))
}

/// Classify the intent of a natural language question.
fn classify_intent(lower: &str) -> Intent {
    // Check for caller-related queries
    if lower.contains("what calls")
        || lower.contains("who calls")
        || lower.contains("callers of")
        || lower.contains("called by")
    {
        return Intent::TraceCallers;
    }

    // Check for callee-related queries
    if lower.contains("what does") && lower.contains("call")
        || lower.contains("callees of")
        || lower.contains("calls what")
        || lower.contains("dependencies of")
    {
        return Intent::TraceCallees;
    }

    // Check for blast radius / impact queries
    if lower.contains("what breaks")
        || lower.contains("impact of")
        || lower.contains("blast radius")
        || lower.contains("affected by")
        || lower.contains("what happens if i change")
        || lower.contains("what happens if I change")
    {
        return Intent::BlastRadius;
    }

    // Check for security queries
    if lower.contains("security")
        || lower.contains("taint")
        || lower.contains("vulnerability")
        || lower.contains("vulnerabilities")
        || lower.contains("owasp")
        || lower.contains("injection")
    {
        return Intent::Security;
    }

    // Check for dead code queries
    if lower.contains("dead code")
        || lower.contains("unused")
        || lower.contains("unreachable")
        || lower.contains("no callers")
    {
        return Intent::DeadCode;
    }

    // Check for architecture queries (before explain, since "what is the structure" should be architecture)
    if lower.contains("architecture")
        || lower.contains("overview")
        || lower.contains("structure")
        || lower.contains("modules")
        || lower.contains("high level")
        || lower.contains("high-level")
    {
        return Intent::Architecture;
    }

    // Check for explain queries
    if lower.contains("explain")
        || lower.contains("what is")
        || lower.contains("describe")
        || lower.contains("tell me about")
    {
        return Intent::Explain;
    }

    // Check for search/find queries
    if lower.contains("find")
        || lower.contains("where is")
        || lower.contains("search")
        || lower.contains("locate")
        || lower.contains("look for")
    {
        return Intent::Search;
    }

    Intent::Fallback
}

/// Extract a symbol name or FQN from the question.
///
/// Looks for patterns like:
/// - Backtick-quoted identifiers: `some_function`
/// - Double-colon paths: module::function
/// - CamelCase identifiers
/// - snake_case identifiers after keywords
fn extract_symbol(question: &str) -> Option<String> {
    // First, check for backtick-quoted identifiers
    if let Some(start) = question.find('`') {
        if let Some(end) = question[start + 1..].find('`') {
            let symbol = &question[start + 1..start + 1 + end];
            if !symbol.is_empty() {
                return Some(symbol.to_string());
            }
        }
    }

    // Check for double-colon paths (FQNs like src/main.rs::function)
    for word in question.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric() && c != ':' && c != '/' && c != '.' && c != '_' && c != '-');
        if cleaned.contains("::") && cleaned.len() > 3 {
            return Some(cleaned.to_string());
        }
    }

    // Look for identifiers after keywords like "calls", "of", "is", "about"
    let keywords = ["calls", "of", "is", "about", "change", "find", "explain", "for"];
    let words: Vec<&str> = question.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let lower_word = word.to_lowercase();
        if keywords.contains(&lower_word.as_str()) && i + 1 < words.len() {
            let candidate = words[i + 1].trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':' && c != '/' && c != '.');
            if candidate.len() >= 2 && (candidate.contains('_') || candidate.contains("::") || candidate.contains('.') || candidate.chars().any(|c| c.is_uppercase())) {
                return Some(candidate.to_string());
            }
        }
    }

    // Look for any snake_case or CamelCase identifier
    for word in question.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':' && c != '/' && c != '.');
        if cleaned.len() >= 3 {
            let has_underscore = cleaned.contains('_');
            let has_mixed_case = cleaned.chars().any(|c| c.is_uppercase()) && cleaned.chars().any(|c| c.is_lowercase()) && cleaned.len() > 3;
            let has_path_sep = cleaned.contains("::") || cleaned.contains('/');
            if has_underscore || has_mixed_case || has_path_sep {
                // Skip common English words that happen to have mixed case
                let skip_words = ["What", "Where", "How", "When", "Why", "The", "This", "That"];
                if !skip_words.contains(&cleaned) {
                    return Some(cleaned.to_string());
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_intent_callers() {
        assert_eq!(classify_intent("what calls validate_token"), Intent::TraceCallers);
        assert_eq!(classify_intent("who calls this function"), Intent::TraceCallers);
        assert_eq!(classify_intent("callers of main"), Intent::TraceCallers);
    }

    #[test]
    fn test_classify_intent_callees() {
        assert_eq!(classify_intent("what does main call"), Intent::TraceCallees);
        assert_eq!(classify_intent("callees of dispatch_tool"), Intent::TraceCallees);
    }

    #[test]
    fn test_classify_intent_blast_radius() {
        assert_eq!(classify_intent("what breaks if i change dispatch_tool"), Intent::BlastRadius);
        assert_eq!(classify_intent("impact of removing validate_token"), Intent::BlastRadius);
        assert_eq!(classify_intent("blast radius of main"), Intent::BlastRadius);
    }

    #[test]
    fn test_classify_intent_explain() {
        assert_eq!(classify_intent("explain dispatch_tool"), Intent::Explain);
        assert_eq!(classify_intent("what is StoreManager"), Intent::Explain);
        assert_eq!(classify_intent("describe the auth module"), Intent::Explain);
    }

    #[test]
    fn test_classify_intent_security() {
        assert_eq!(classify_intent("are there any security issues"), Intent::Security);
        assert_eq!(classify_intent("find taint paths"), Intent::Security);
        assert_eq!(classify_intent("check for vulnerabilities"), Intent::Security);
    }

    #[test]
    fn test_classify_intent_dead_code() {
        assert_eq!(classify_intent("find dead code"), Intent::DeadCode);
        assert_eq!(classify_intent("what functions are unused"), Intent::DeadCode);
    }

    #[test]
    fn test_classify_intent_architecture() {
        assert_eq!(classify_intent("show me the architecture"), Intent::Architecture);
        assert_eq!(classify_intent("give me an overview"), Intent::Architecture);
        assert_eq!(classify_intent("what is the high-level structure"), Intent::Architecture);
    }

    #[test]
    fn test_classify_intent_search() {
        assert_eq!(classify_intent("find dispatch_tool"), Intent::Search);
        assert_eq!(classify_intent("where is the main function"), Intent::Search);
    }

    #[test]
    fn test_classify_intent_fallback() {
        assert_eq!(classify_intent("hello world"), Intent::Fallback);
    }

    #[test]
    fn test_extract_symbol_backtick() {
        assert_eq!(extract_symbol("what calls `validate_token`"), Some("validate_token".to_string()));
        assert_eq!(extract_symbol("explain `src/main.rs::main`"), Some("src/main.rs::main".to_string()));
    }

    #[test]
    fn test_extract_symbol_fqn() {
        assert_eq!(extract_symbol("what calls src/auth.rs::validate_token"), Some("src/auth.rs::validate_token".to_string()));
    }

    #[test]
    fn test_extract_symbol_snake_case() {
        assert_eq!(extract_symbol("explain dispatch_tool please"), Some("dispatch_tool".to_string()));
    }

    #[test]
    fn test_extract_symbol_camel_case() {
        assert_eq!(extract_symbol("explain StoreManager"), Some("StoreManager".to_string()));
    }

    #[test]
    fn test_extract_symbol_none() {
        assert_eq!(extract_symbol("hello"), None);
    }
}
