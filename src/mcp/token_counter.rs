//! Token estimation utilities for MCP tool responses.
//!
//! Provides lightweight heuristics for estimating token usage and savings
//! in tool call responses. These estimates help AI agents understand the
//! efficiency gains from using structural queries vs reading raw files.

use serde::Serialize;

/// Per-component token breakdown showing where tokens are spent in a response.
#[derive(Debug, Clone, Serialize)]
pub struct TokenBreakdown {
    /// Tokens used for node data (FQNs, kinds, file paths, line numbers).
    pub nodes: usize,

    /// Tokens used for edge/relationship data (callers, callees, links).
    pub edges: usize,

    /// Tokens used for code snippets or source content.
    pub code: usize,

    /// Tokens used for metadata, formatting, and structural JSON overhead.
    pub metadata: usize,
}

/// Metadata about token usage attached to every tool response.
#[derive(Debug, Clone, Serialize)]
pub struct TokenMeta {
    /// Estimated tokens in the response (1 token per 4 characters).
    pub tokens_used: usize,

    /// Estimated tokens saved by using the structural query instead of reading raw files.
    pub tokens_saved: usize,

    /// Wall-clock time in milliseconds for the query execution.
    pub query_time_ms: u64,

    /// Per-component token breakdown.
    pub token_breakdown: TokenBreakdown,
}

/// Estimate tokens_used from the response JSON (1 token per 4 characters).
pub fn estimate_tokens_used(response_json: &str) -> usize {
    response_json.len() / 4
}

/// Estimate tokens_saved: sum of estimated tokens in source files touched by query minus tokens_used.
/// For structural queries, estimate based on number of files touched * average file size.
pub fn estimate_tokens_saved(files_touched: usize, tokens_used: usize) -> usize {
    let avg_file_tokens: usize = 750; // ~3000 chars / 4 = 750 tokens per file
    let total_file_tokens = files_touched * avg_file_tokens;
    total_file_tokens.saturating_sub(tokens_used)
}

/// Compute per-component token breakdown from the response JSON.
///
/// Heuristic approach: parse the JSON and categorize content by field names.
/// - "fqn", "kind", "file", "start_line", "end_line", "confidence" → nodes
/// - "caller_fqn", "source_fqn", "target_fqn", "call_count", "depth" → edges
/// - "code", "snippet", "source" → code
/// - Everything else (JSON structure, keys, formatting) → metadata
///
/// The breakdown is guaranteed to sum to tokens_used.
pub fn compute_token_breakdown(response_json: &str, tokens_used: usize) -> TokenBreakdown {
    if tokens_used == 0 {
        return TokenBreakdown {
            nodes: 0,
            edges: 0,
            code: 0,
            metadata: 0,
        };
    }

    // Parse JSON to estimate component sizes
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(response_json);

    let (nodes_chars, edges_chars, code_chars) = match parsed {
        Ok(value) => count_component_chars(&value),
        Err(_) => (0usize, 0usize, 0usize),
    };

    let total_chars = response_json.len();

    // Convert char counts to token proportions
    if total_chars == 0 {
        return TokenBreakdown {
            nodes: 0,
            edges: 0,
            code: 0,
            metadata: tokens_used,
        };
    }

    let nodes_tokens = (nodes_chars as f64 / total_chars as f64 * tokens_used as f64) as usize;
    let edges_tokens = (edges_chars as f64 / total_chars as f64 * tokens_used as f64) as usize;
    let code_tokens = (code_chars as f64 / total_chars as f64 * tokens_used as f64) as usize;

    // Metadata gets the remainder to ensure exact sum
    let metadata_tokens = tokens_used.saturating_sub(nodes_tokens + edges_tokens + code_tokens);

    TokenBreakdown {
        nodes: nodes_tokens,
        edges: edges_tokens,
        code: code_tokens,
        metadata: metadata_tokens,
    }
}

/// Recursively count characters belonging to node, edge, and code components.
fn count_component_chars(value: &serde_json::Value) -> (usize, usize, usize) {
    let mut nodes = 0usize;
    let mut edges = 0usize;
    let mut code = 0usize;

    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let val_len = val.to_string().len();
                match key.as_str() {
                    // Node fields
                    "fqn" | "kind" | "file" | "start_line" | "end_line" | "confidence"
                    | "node_count" | "complexity" | "language" => {
                        nodes += val_len;
                    }
                    // Edge fields
                    "caller_fqn" | "source_fqn" | "target_fqn" | "call_count" | "depth"
                    | "internal_edges" | "external_edges" | "blast_radius" | "edge_attributes" => {
                        edges += val_len;
                    }
                    // Code fields
                    "code" | "snippet" | "source" => {
                        code += val_len;
                    }
                    // Recurse into nested structures
                    _ => {
                        let (n, e, c) = count_component_chars(val);
                        nodes += n;
                        edges += e;
                        code += c;
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                let (n, e, c) = count_component_chars(item);
                nodes += n;
                edges += e;
                code += c;
            }
        }
        _ => {}
    }

    (nodes, edges, code)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_used_basic() {
        // 100 chars -> 25 tokens
        let input = "a".repeat(100);
        assert_eq!(estimate_tokens_used(&input), 25);
    }

    #[test]
    fn test_estimate_tokens_used_empty() {
        assert_eq!(estimate_tokens_used(""), 0);
    }

    #[test]
    fn test_estimate_tokens_used_short() {
        // 3 chars -> 0 tokens (integer division)
        assert_eq!(estimate_tokens_used("abc"), 0);
    }

    #[test]
    fn test_estimate_tokens_saved_basic() {
        // 10 files * 750 tokens = 7500, minus 100 used = 7400 saved
        assert_eq!(estimate_tokens_saved(10, 100), 7400);
    }

    #[test]
    fn test_estimate_tokens_saved_zero_files() {
        assert_eq!(estimate_tokens_saved(0, 100), 0);
    }

    #[test]
    fn test_estimate_tokens_saved_saturating() {
        // 1 file * 750 = 750, minus 1000 used -> saturates to 0
        assert_eq!(estimate_tokens_saved(1, 1000), 0);
    }

    #[test]
    fn test_token_meta_serializes() {
        let meta = TokenMeta {
            tokens_used: 150,
            tokens_saved: 5000,
            query_time_ms: 12,
            token_breakdown: TokenBreakdown {
                nodes: 80,
                edges: 40,
                code: 0,
                metadata: 30,
            },
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["tokens_used"], 150);
        assert_eq!(json["tokens_saved"], 5000);
        assert_eq!(json["query_time_ms"], 12);
        assert_eq!(json["token_breakdown"]["nodes"], 80);
        assert_eq!(json["token_breakdown"]["edges"], 40);
        assert_eq!(json["token_breakdown"]["code"], 0);
        assert_eq!(json["token_breakdown"]["metadata"], 30);
    }

    #[test]
    fn test_token_breakdown_sums_to_tokens_used() {
        let response = r#"[{"fqn":"src/main.rs::main","kind":"Function","file":"src/main.rs","start_line":1,"end_line":10,"confidence":1.0}]"#;
        let tokens_used = estimate_tokens_used(response);
        let breakdown = compute_token_breakdown(response, tokens_used);

        let sum = breakdown.nodes + breakdown.edges + breakdown.code + breakdown.metadata;
        assert_eq!(sum, tokens_used, "Breakdown must sum to tokens_used");
    }

    #[test]
    fn test_token_breakdown_empty_response() {
        let breakdown = compute_token_breakdown("[]", 0);
        assert_eq!(breakdown.nodes, 0);
        assert_eq!(breakdown.edges, 0);
        assert_eq!(breakdown.code, 0);
        assert_eq!(breakdown.metadata, 0);
    }

    #[test]
    fn test_token_breakdown_code_heavy_response() {
        let response = r#"{"fqn":"test::func","code":"fn main() {\n    println!(\"hello world\");\n    let x = 42;\n    let y = x * 2;\n}"}"#;
        let tokens_used = estimate_tokens_used(response);
        let breakdown = compute_token_breakdown(response, tokens_used);

        let sum = breakdown.nodes + breakdown.edges + breakdown.code + breakdown.metadata;
        assert_eq!(sum, tokens_used, "Breakdown must sum to tokens_used");
        // Code should be a significant portion
        assert!(
            breakdown.code > 0,
            "Expected code tokens > 0 for code-heavy response"
        );
    }

    #[test]
    fn test_token_breakdown_edge_heavy_response() {
        let response = r#"[{"fqn":"a::b","caller_fqn":"c::d","call_count":5,"depth":2},{"fqn":"e::f","caller_fqn":"g::h","call_count":3,"depth":1}]"#;
        let tokens_used = estimate_tokens_used(response);
        let breakdown = compute_token_breakdown(response, tokens_used);

        let sum = breakdown.nodes + breakdown.edges + breakdown.code + breakdown.metadata;
        assert_eq!(sum, tokens_used, "Breakdown must sum to tokens_used");
        assert!(
            breakdown.edges > 0,
            "Expected edge tokens > 0 for edge-heavy response"
        );
    }
}
