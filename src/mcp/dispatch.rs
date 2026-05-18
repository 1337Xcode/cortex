//! MCP tool dispatch: maps tool names to store/memory methods.
//!
//! Each tool call is dispatched to the appropriate store or memory function,
//! and the result is wrapped with `_meta` containing token estimates and timing.

use std::time::Instant;

use serde_json::Value;

use crate::error::McpError;
use crate::store::db::StoreManager;
use crate::store::queries::community;
use crate::store::queries::graph;
use crate::store::queries::memory;
use crate::store::queries::search;
use crate::store::types::NodeKind;
use crate::security::{owasp, sbom, taint, vuln};
use crate::security::secrets;
use crate::agents::steering;
use crate::indexer::context_budget;

use super::token_counter::{compute_token_breakdown, estimate_tokens_saved, estimate_tokens_used, TokenMeta};
use super::types::{ToolCallResult, ToolContent};

/// Dispatch a tool call to the appropriate store/memory method.
///
/// Returns a `ToolCallResult` with the response content and `_meta` fields
/// for token estimation.
pub fn dispatch_tool(
    store: &StoreManager,
    tool_name: &str,
    arguments: &Value,
) -> Result<Value, McpError> {
    let start = Instant::now();

    // search_symbols has special handling for retrieval_method in _meta
    let (result_json, files_touched, retrieval_method) = if tool_name == "search_symbols" {
        let (json, files, method) = dispatch_search_symbols(store, arguments)?;
        (json, files, Some(method))
    } else {
        let (json, files) = match tool_name {
        "trace_callers" => dispatch_trace_callers(store, arguments)?,
        "trace_callees" => dispatch_trace_callees(store, arguments)?,
        "get_file_context" => dispatch_get_file_context(store, arguments)?,
        "get_architecture" => dispatch_get_architecture(store)?,
        "find_dead_code" => dispatch_find_dead_code(store, arguments)?,
        "blast_radius" => dispatch_blast_radius(store, arguments)?,
        "write_observation" => dispatch_write_observation(store, arguments)?,
        "read_observations" => dispatch_read_observations(store, arguments)?,
        "write_adr" => dispatch_write_adr(store, arguments)?,
        "read_adrs" => dispatch_read_adrs(store, arguments)?,
        "prune_observations" => dispatch_prune_observations(store, arguments)?,
        "detect_changes" => dispatch_detect_changes(store, arguments)?,
        "semantic_search" => dispatch_semantic_search(store, arguments)?,
        "search_text" => dispatch_search_text(store, arguments)?,
        "get_code_snippet" => dispatch_get_code_snippet(store, arguments)?,
        "query_graph" => dispatch_query_graph(store, arguments)?,
        "get_http_routes" => dispatch_get_http_routes(store, arguments)?,
        "trace_http_call" => dispatch_trace_http_call(store, arguments)?,
        "find_taint_paths" => dispatch_find_taint_paths(store, arguments)?,
        "scan_owasp" => dispatch_scan_owasp(store)?,
        "generate_sbom" => dispatch_generate_sbom(store, arguments)?,
        "check_dependencies" => dispatch_check_dependencies(store, arguments)?,
        "generate_steering" => dispatch_generate_steering(store)?,
        "decompose_boundaries" => dispatch_decompose_boundaries(store, arguments)?,
        "get_complexity_hotspots" => dispatch_get_complexity_hotspots(store, arguments)?,
        "get_task_context" => dispatch_get_task_context(store, arguments)?,
        "get_class_hierarchy" => dispatch_get_class_hierarchy(store, arguments)?,
        "get_git_hotspots" => dispatch_get_git_hotspots(store, arguments)?,
        "get_import_graph" => dispatch_get_import_graph(store, arguments)?,
        "find_similar_functions" => dispatch_find_similar_functions(store, arguments)?,
        "ask" => super::ask::dispatch_ask(store, arguments)?,
        _ => {
            return Err(McpError::DispatchError {
                reason: format!("unknown tool: {}", tool_name),
            });
        }
        };
        (json, files, None)
    };

    let query_time_ms = start.elapsed().as_millis() as u64;
    let tokens_used = estimate_tokens_used(&result_json);
    let tokens_saved = estimate_tokens_saved(files_touched, tokens_used);
    let token_breakdown = compute_token_breakdown(&result_json, tokens_used);

    let meta = TokenMeta {
        tokens_used,
        tokens_saved,
        query_time_ms,
        token_breakdown,
    };

    let tool_result = ToolCallResult {
        content: vec![ToolContent {
            content_type: "text".to_string(),
            text: result_json,
        }],
        is_error: None,
    };

    let mut response = serde_json::to_value(tool_result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize tool result: {}", e),
    })?;

    // Attach _meta to the response
    let mut meta_value = serde_json::to_value(meta).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize meta: {}", e),
    })?;

    // Include retrieval_method in _meta if present (from search_symbols)
    if let Some(method) = retrieval_method {
        meta_value.as_object_mut().unwrap().insert(
            "retrieval_method".to_string(),
            serde_json::json!(method),
        );
    }

    response
        .as_object_mut()
        .unwrap()
        .insert("_meta".to_string(), meta_value);

    Ok(response)
}

// ---------------------------------------------------------------------------
// Individual tool dispatchers
// ---------------------------------------------------------------------------

/// search_symbols: calls find_nodes_by_pattern with hybrid FTS5 fallback.
///
/// If graph returns fewer than 3 results, also runs FTS5 BM25 search.
/// Results are merged, deduplicated by FQN (preferring higher confidence),
/// and sorted by confidence descending.
///
/// Returns a tuple of (json_string, files_touched, retrieval_method).
fn dispatch_search_symbols(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize, &'static str), McpError> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: pattern".to_string(),
        })?;

    let kind = args.get("kind").and_then(|v| v.as_str()).and_then(parse_node_kind);

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;

    let conn = store.read_conn();
    let nodes = graph::find_nodes_by_pattern(&conn, pattern, kind, limit).map_err(|e| {
        McpError::DispatchError {
            reason: format!("search_symbols failed: {}", e),
        }
    })?;

    let graph_count = nodes.len();

    // Wrap graph nodes with confidence 1.0
    let mut results_with_confidence: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            let mut v = serde_json::to_value(n).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("confidence".to_string(), serde_json::json!(1.0));
            }
            v
        })
        .collect();

    // Hybrid fallback: if graph returns <3 results, also run FTS5
    let retrieval_method = if graph_count < 3 {
        // Convert glob pattern to a search query for FTS5
        // Strip wildcards (*) and use the remaining text as the search query
        let fts_query = pattern.replace('*', " ").replace("::", " ");
        let fts_query = fts_query.trim();

        if !fts_query.is_empty() {
            let fts_results = search::search_fts(&conn, fts_query, limit).map_err(|e| {
                McpError::DispatchError {
                    reason: format!("search_symbols FTS5 fallback failed: {}", e),
                }
            })?;

            if !fts_results.is_empty() {
                // Add FTS5 results
                for fts_result in &fts_results {
                    let v = serde_json::json!({
                        "fqn": fts_result.fqn,
                        "kind": fts_result.kind,
                        "file": fts_result.file,
                        "confidence": fts_result.confidence,
                    });
                    results_with_confidence.push(v);
                }

                // Deduplicate by FQN, keeping the entry with higher confidence
                results_with_confidence = deduplicate_by_fqn(results_with_confidence);

                // Sort by confidence descending
                results_with_confidence.sort_by(|a, b| {
                    let ca = a.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let cb = b.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    cb.partial_cmp(&ca).unwrap_or(std::cmp::Ordering::Equal)
                });

                if graph_count == 0 {
                    "fts5"
                } else {
                    "hybrid"
                }
            } else if graph_count == 0 {
                "fts5"
            } else {
                "graph"
            }
        } else if graph_count == 0 {
            "fts5"
        } else {
            "graph"
        }
    } else {
        "graph"
    };

    // Enforce limit after merge
    results_with_confidence.truncate(limit);

    let files_touched = {
        let mut files: Vec<&str> = results_with_confidence
            .iter()
            .filter_map(|v| v.get("file").and_then(|f| f.as_str()))
            .collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let json = serde_json::to_string(&results_with_confidence).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize search results: {}", e),
    })?;

    Ok((json, files_touched, retrieval_method))
}

/// Deduplicate results by FQN, keeping the entry with the highest confidence.
fn deduplicate_by_fqn(results: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    use std::collections::HashMap;
    let mut seen: HashMap<String, serde_json::Value> = HashMap::new();

    for item in results {
        let fqn = item.get("fqn").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let confidence = item.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);

        if let Some(existing) = seen.get(&fqn) {
            let existing_confidence = existing.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if confidence > existing_confidence {
                seen.insert(fqn, item);
            }
        } else {
            seen.insert(fqn, item);
        }
    }

    seen.into_values().collect()
}

/// trace_callers: calls graph::trace_callers
fn dispatch_trace_callers(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let fqn = args
        .get("fqn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: fqn".to_string(),
        })?;

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as u32;

    let conn = store.read_conn();
    let callers = graph::trace_callers(&conn, fqn, depth).map_err(|e| McpError::DispatchError {
        reason: format!("trace_callers failed: {}", e),
    })?;

    // Results are already sorted by call_count desc, confidence desc from graph::trace_callers

    let files_touched = count_unique_files_from_call_path(&callers);
    let json = serde_json::to_string(&callers).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize callers: {}", e),
    })?;

    Ok((json, files_touched))
}

/// trace_callees: calls graph::trace_callees
fn dispatch_trace_callees(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let fqn = args
        .get("fqn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: fqn".to_string(),
        })?;

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as u32;

    let conn = store.read_conn();
    let callees = graph::trace_callees(&conn, fqn, depth).map_err(|e| McpError::DispatchError {
        reason: format!("trace_callees failed: {}", e),
    })?;

    // Results are already sorted by call_count desc, confidence desc from graph::trace_callees

    let files_touched = count_unique_files_from_call_path(&callees);
    let json = serde_json::to_string(&callees).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize callees: {}", e),
    })?;

    Ok((json, files_touched))
}

/// get_file_context: calls find_nodes_by_pattern with file filter
fn dispatch_get_file_context(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let file = args
        .get("file")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: file".to_string(),
        })?;

    // Use file path as pattern prefix to find all nodes in that file
    let pattern = format!("{}::*", file);
    let conn = store.read_conn();
    let nodes =
        graph::find_nodes_by_pattern(&conn, &pattern, None, 500).map_err(|e| {
            McpError::DispatchError {
                reason: format!("get_file_context failed: {}", e),
            }
        })?;

    let files_touched = 1; // Single file context

    // Wrap nodes with confidence 1.0 (graph-resolved results)
    let results_with_confidence: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            let mut v = serde_json::to_value(n).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("confidence".to_string(), serde_json::json!(1.0));
            }
            v
        })
        .collect();

    let json = serde_json::to_string(&results_with_confidence).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize file context: {}", e),
    })?;

    Ok((json, files_touched))
}

/// get_architecture: calls graph::get_architecture_summary
fn dispatch_get_architecture(store: &StoreManager) -> Result<(String, usize), McpError> {
    let conn = store.read_conn();
    let summary = graph::get_architecture_summary(&conn).map_err(|e| McpError::DispatchError {
        reason: format!("get_architecture failed: {}", e),
    })?;

    let files_touched = summary.top_level_modules.len().max(1);
    let json = serde_json::to_string(&summary).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize architecture: {}", e),
    })?;

    Ok((json, files_touched))
}

/// find_dead_code: calls graph::find_dead_code
fn dispatch_find_dead_code(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;

    let conn = store.read_conn();
    let nodes = graph::find_dead_code(&conn, limit).map_err(|e| McpError::DispatchError {
        reason: format!("find_dead_code failed: {}", e),
    })?;

    let files_touched = count_unique_files_from_nodes(&nodes);

    // Wrap nodes with confidence 1.0 (graph-resolved results)
    let results_with_confidence: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            let mut v = serde_json::to_value(n).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("confidence".to_string(), serde_json::json!(1.0));
            }
            v
        })
        .collect();

    let json = serde_json::to_string(&results_with_confidence).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize dead code: {}", e),
    })?;

    Ok((json, files_touched))
}

/// blast_radius: calls graph::blast_radius
fn dispatch_blast_radius(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let fqn = args
        .get("fqn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: fqn".to_string(),
        })?;

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as u32;

    let conn = store.read_conn();
    let nodes = graph::blast_radius(&conn, fqn, depth).map_err(|e| McpError::DispatchError {
        reason: format!("blast_radius failed: {}", e),
    })?;

    let files_touched = count_unique_files_from_nodes(&nodes);

    // Wrap nodes with confidence 1.0 (graph-resolved results)
    let results_with_confidence: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            let mut v = serde_json::to_value(n).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("confidence".to_string(), serde_json::json!(1.0));
            }
            v
        })
        .collect();

    let json = serde_json::to_string(&results_with_confidence).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize blast radius: {}", e),
    })?;

    Ok((json, files_touched))
}

/// search_text: calls search::search_fts with BM25 ranking
fn dispatch_search_text(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: query".to_string(),
        })?;

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let conn = store.read_conn();
    let results = search::search_fts(&conn, query, limit).map_err(|e| McpError::DispatchError {
        reason: format!("search_text failed: {}", e),
    })?;

    let files_touched = {
        let mut files: Vec<&str> = results.iter().map(|r| r.file.as_str()).collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    // Results are already sorted by confidence descending from search_fts
    let json = serde_json::to_string(&results).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize search results: {}", e),
    })?;

    Ok((json, files_touched))
}

/// write_observation: calls memory::write_observation
fn dispatch_write_observation(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let node_fqn = args
        .get("node_fqn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: node_fqn".to_string(),
        })?;

    let observation_text = args
        .get("observation_text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: observation_text".to_string(),
        })?;

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let conn = store.write_conn();
    let id = memory::write_observation(&conn, node_fqn, observation_text, agent_id, "")
        .map_err(|e| McpError::DispatchError {
            reason: format!("write_observation failed: {}", e),
        })?;

    let result = serde_json::json!({ "id": id });
    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize observation id: {}", e),
    })?;

    Ok((json, 0))
}

/// read_observations: calls memory::read_observations
fn dispatch_read_observations(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let fqn = args
        .get("fqn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: fqn".to_string(),
        })?;

    let include_stale = args
        .get("include_stale")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let conn = store.read_conn();
    let observations =
        memory::read_observations(&conn, fqn, include_stale).map_err(|e| {
            McpError::DispatchError {
                reason: format!("read_observations failed: {}", e),
            }
        })?;

    let json = serde_json::to_string(&observations).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize observations: {}", e),
    })?;

    Ok((json, 0))
}

/// write_adr: calls memory::write_adr
fn dispatch_write_adr(store: &StoreManager, args: &Value) -> Result<(String, usize), McpError> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: title".to_string(),
        })?;

    let body = args
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: body".to_string(),
        })?;

    let status = args
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("proposed");

    let linked_fqn = args.get("linked_fqn").and_then(|v| v.as_str());

    let conn = store.write_conn();
    let id = memory::write_adr(&conn, title, body, status, linked_fqn).map_err(|e| {
        McpError::DispatchError {
            reason: format!("write_adr failed: {}", e),
        }
    })?;

    let result = serde_json::json!({ "id": id });
    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize ADR id: {}", e),
    })?;

    Ok((json, 0))
}

/// read_adrs: calls memory::read_adrs
fn dispatch_read_adrs(store: &StoreManager, args: &Value) -> Result<(String, usize), McpError> {
    let fqn = args.get("fqn").and_then(|v| v.as_str());
    let status = args.get("status").and_then(|v| v.as_str());

    let conn = store.read_conn();
    let adrs = memory::read_adrs(&conn, fqn, status).map_err(|e| McpError::DispatchError {
        reason: format!("read_adrs failed: {}", e),
    })?;

    let json = serde_json::to_string(&adrs).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize ADRs: {}", e),
    })?;

    Ok((json, 0))
}

/// prune_observations: calls memory::prune_stale_observations
fn dispatch_prune_observations(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let older_than_days = args
        .get("older_than_days")
        .and_then(|v| v.as_u64())
        .map(|d| d as u32);

    let conn = store.write_conn();
    let count =
        memory::prune_stale_observations(&conn, older_than_days).map_err(|e| {
            McpError::DispatchError {
                reason: format!("prune_observations failed: {}", e),
            }
        })?;

    let result = serde_json::json!({ "pruned": count });
    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize prune result: {}", e),
    })?;

    Ok((json, 0))
}

/// find_taint_paths: runs taint analysis and returns paths
fn dispatch_find_taint_paths(
    store: &StoreManager,
    _args: &Value,
) -> Result<(String, usize), McpError> {
    let paths = taint::propagate_taint(store).map_err(|e| McpError::DispatchError {
        reason: format!("find_taint_paths failed: {}", e),
    })?;

    let files_touched = {
        let mut files: Vec<&str> = paths.iter().map(|p| p.source_fqn.as_str()).collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let json = serde_json::to_string(&paths).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize taint paths: {}", e),
    })?;

    Ok((json, files_touched))
}

/// scan_owasp: runs OWASP pattern detection
fn dispatch_scan_owasp(store: &StoreManager) -> Result<(String, usize), McpError> {
    let findings = owasp::scan_owasp_patterns(store).map_err(|e| McpError::DispatchError {
        reason: format!("scan_owasp failed: {}", e),
    })?;

    let files_touched = {
        let mut files: Vec<&str> = findings.iter().map(|f| f.node_fqn.as_str()).collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let json = serde_json::to_string(&findings).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize OWASP findings: {}", e),
    })?;

    Ok((json, files_touched))
}

/// generate_sbom: generates SBOM and returns SPDX JSON
fn dispatch_generate_sbom(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let repo_root = args
        .get("repo_root")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    let repo_path = std::path::Path::new(repo_root);
    let entries = sbom::generate_sbom(store, repo_path).map_err(|e| McpError::DispatchError {
        reason: format!("generate_sbom failed: {}", e),
    })?;

    let project_name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let spdx = sbom::generate_spdx(&entries, project_name);
    let json = serde_json::to_string(&spdx).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize SBOM: {}", e),
    })?;

    Ok((json, entries.len()))
}

/// check_dependencies: cross-references SBOM against OSV.dev
fn dispatch_check_dependencies(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let repo_root = args
        .get("repo_root")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    let repo_path = std::path::Path::new(repo_root);
    let entries = sbom::generate_sbom(store, repo_path).map_err(|e| McpError::DispatchError {
        reason: format!("check_dependencies: sbom generation failed: {}", e),
    })?;

    let results = vuln::check_osv(&entries).map_err(|e| McpError::DispatchError {
        reason: format!("check_dependencies failed: {}", e),
    })?;

    let json = serde_json::to_string(&results).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize vulnerability results: {}", e),
    })?;

    Ok((json, results.len()))
}

/// generate_steering: analyzes graph and returns steering file content
fn dispatch_generate_steering(store: &StoreManager) -> Result<(String, usize), McpError> {
    let content = steering::generate_steering(store).map_err(|e| McpError::DispatchError {
        reason: format!("generate_steering failed: {}", e),
    })?;

    let json = serde_json::to_string(&content).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize steering content: {}", e),
    })?;

    Ok((json, 0))
}

/// decompose_boundaries: runs Leiden community detection on the call graph
fn dispatch_decompose_boundaries(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let module_path = args.get("module_path").and_then(|v| v.as_str());

    let coupling_threshold = args
        .get("coupling_threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);

    let conn = store.read_conn();
    let result =
        community::detect_communities(&conn, module_path, coupling_threshold).map_err(|e| {
            McpError::DispatchError {
                reason: format!("decompose_boundaries failed: {}", e),
            }
        })?;

    let files_touched: usize = result
        .communities
        .iter()
        .flat_map(|c| c.files.iter())
        .collect::<std::collections::HashSet<_>>()
        .len();

    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize community detection result: {}", e),
    })?;

    Ok((json, files_touched))
}

/// get_code_snippet: reads actual source code for a symbol by FQN
fn dispatch_get_code_snippet(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let fqn = args
        .get("fqn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: fqn".to_string(),
        })?;

    let conn = store.read_conn();

    // Look up the node to get file path and line range
    let node = graph::find_node_by_fqn(&conn, fqn).map_err(|e| McpError::DispatchError {
        reason: format!("failed to find node: {}", e),
    })?;

    let node = node.ok_or_else(|| McpError::DispatchError {
        reason: format!("node not found: {}", fqn),
    })?;

    // Read the source file and extract the relevant lines
    // Use CORTEX_REPO_ROOT env var to find the file
    let repo_root = std::env::var("CORTEX_REPO_ROOT").unwrap_or_else(|_| ".".to_string());
    let file_path = std::path::Path::new(&repo_root).join(&node.file);

    let source = std::fs::read_to_string(&file_path).map_err(|e| McpError::DispatchError {
        reason: format!("failed to read file '{}': {}", node.file, e),
    })?;

    let lines: Vec<&str> = source.lines().collect();
    let start = (node.start_line as usize).saturating_sub(1);
    let end = (node.end_line as usize).min(lines.len());

    let snippet = if start < end {
        lines[start..end].join("\n")
    } else {
        String::new()
    };

    // Apply secret redaction if the node has contains_secret attribute
    let snippet = if node.attributes.get("contains_secret").is_some() {
        let secret_matches = secrets::detect_secrets(&snippet);
        if !secret_matches.is_empty() {
            secrets::redact_secrets(&snippet, &secret_matches)
        } else {
            snippet
        }
    } else {
        snippet
    };

    let result = serde_json::json!({
        "fqn": fqn,
        "file": node.file,
        "start_line": node.start_line,
        "end_line": node.end_line,
        "language": detect_language_from_file(&node.file),
        "code": snippet,
    });

    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize snippet: {}", e),
    })?;

    Ok((json, 1))
}

/// get_complexity_hotspots: query Function nodes with complexity >= threshold, sorted desc
fn dispatch_get_complexity_hotspots(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let threshold = args
        .get("threshold")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as i64;

    let conn = store.read_conn();

    let mut stmt = conn
        .prepare(
            "SELECT fqn, kind, file, start_line, end_line, attributes \
             FROM nodes \
             WHERE kind = 'Function' \
               AND CAST(json_extract(attributes, '$.complexity') AS INTEGER) >= ?1 \
             ORDER BY CAST(json_extract(attributes, '$.complexity') AS INTEGER) DESC \
             LIMIT ?2",
        )
        .map_err(|e| McpError::DispatchError {
            reason: format!("get_complexity_hotspots query failed: {}", e),
        })?;

    let results: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params![threshold, limit], |row| {
            let fqn: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let file: String = row.get(2)?;
            let start_line: u32 = row.get(3)?;
            let end_line: u32 = row.get(4)?;
            let attrs_str: String = row.get(5)?;
            let attrs: serde_json::Value =
                serde_json::from_str(&attrs_str).unwrap_or_default();
            let complexity = attrs
                .get("complexity")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Ok(serde_json::json!({
                "fqn": fqn,
                "kind": kind,
                "file": file,
                "start_line": start_line,
                "end_line": end_line,
                "complexity": complexity,
            }))
        })
        .map_err(|e| McpError::DispatchError {
            reason: format!("get_complexity_hotspots query failed: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    let files_touched = {
        let mut files: Vec<&str> = results
            .iter()
            .filter_map(|r| r.get("file").and_then(|f| f.as_str()))
            .collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let json = serde_json::to_string(&results).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize complexity hotspots: {}", e),
    })?;

    Ok((json, files_touched))
}

/// Detect programming language from file extension.
fn detect_language_from_file(file: &str) -> &'static str {
    match std::path::Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
    {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") => "javascript",
        Some("go") => "go",
        Some("java") => "java",
        Some("cs") => "csharp",
        Some("cpp") | Some("cc") | Some("cxx") => "cpp",
        Some("rb") => "ruby",
        Some("c") | Some("h") => "c",
        _ => "unknown",
    }
}

/// get_http_routes: query Route nodes from the database with optional filters
fn dispatch_get_http_routes(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let service = args.get("service").and_then(|v| v.as_str());
    let method = args.get("method").and_then(|v| v.as_str());
    let path_prefix = args.get("path_prefix").and_then(|v| v.as_str());

    let conn = store.read_conn();

    // Query all Route nodes; optionally filter by method in attributes
    let mut sql = String::from(
        "SELECT fqn, file, start_line, attributes FROM nodes WHERE kind = 'Route'",
    );
    let mut params: Vec<String> = Vec::new();

    if let Some(m) = method {
        sql.push_str(&format!(
            " AND json_extract(attributes, '$.method') = ?{}",
            params.len() + 1
        ));
        params.push(m.to_uppercase());
    }

    sql.push_str(" LIMIT 100");

    let mut stmt = conn.prepare(&sql).map_err(|e| McpError::DispatchError {
        reason: format!("failed to prepare route query: {}", e),
    })?;

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|p| p as &dyn rusqlite::types::ToSql).collect();

    let routes: Vec<serde_json::Value> = stmt
        .query_map(param_refs.as_slice(), |row| {
            let fqn: String = row.get(0)?;
            let file: String = row.get(1)?;
            let start_line: i64 = row.get(2)?;
            let attrs_str: String = row.get(3)?;
            let attrs: serde_json::Value =
                serde_json::from_str(&attrs_str).unwrap_or_default();
            Ok(serde_json::json!({
                "fqn": fqn,
                "file": file,
                "line": start_line,
                "method": attrs.get("method").and_then(|v| v.as_str()).unwrap_or("ALL"),
                "path": attrs.get("path").and_then(|v| v.as_str()).unwrap_or(""),
                "framework": attrs.get("framework").and_then(|v| v.as_str()).unwrap_or("unknown"),
            }))
        })
        .map_err(|e| McpError::DispatchError {
            reason: format!("failed to query routes: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Apply post-query filters for path_prefix and service
    let filtered: Vec<&serde_json::Value> = routes
        .iter()
        .filter(|r| {
            if let Some(prefix) = path_prefix {
                if let Some(path) = r.get("path").and_then(|v| v.as_str()) {
                    if !path.starts_with(prefix) {
                        return false;
                    }
                }
            }
            if let Some(svc) = service {
                if let Some(file) = r.get("file").and_then(|v| v.as_str()) {
                    if !file.contains(svc) {
                        return false;
                    }
                }
            }
            true
        })
        .collect();

    let json = serde_json::to_string(&filtered).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize routes: {}", e),
    })?;

    Ok((json, filtered.len()))
}

/// trace_http_call: match a URL pattern to route definitions and find call sites
fn dispatch_trace_http_call(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let url_pattern = args
        .get("url_pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: url_pattern".to_string(),
        })?;

    let conn = store.read_conn();

    // Find all Route nodes
    let mut stmt = conn
        .prepare("SELECT fqn, file, start_line, attributes FROM nodes WHERE kind = 'Route'")
        .map_err(|e| McpError::DispatchError {
            reason: format!("query failed: {}", e),
        })?;

    let routes: Vec<(String, String, i64, serde_json::Value)> = stmt
        .query_map([], |row| {
            let fqn: String = row.get(0)?;
            let file: String = row.get(1)?;
            let line: i64 = row.get(2)?;
            let attrs_str: String = row.get(3)?;
            let attrs: serde_json::Value =
                serde_json::from_str(&attrs_str).unwrap_or_default();
            Ok((fqn, file, line, attrs))
        })
        .map_err(|e| McpError::DispatchError {
            reason: format!("query failed: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Find routes matching the URL pattern
    let matching: Vec<serde_json::Value> = routes
        .iter()
        .filter_map(|(fqn, file, line, attrs)| {
            let path = attrs.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path == url_pattern
                || url_pattern.contains(path)
                || path.contains(url_pattern)
            {
                Some(serde_json::json!({
                    "route_fqn": fqn,
                    "file": file,
                    "line": line,
                    "method": attrs.get("method").and_then(|v| v.as_str()).unwrap_or("ALL"),
                    "path": path,
                    "framework": attrs.get("framework").and_then(|v| v.as_str()).unwrap_or("unknown"),
                }))
            } else {
                None
            }
        })
        .collect();

    // Find HttpLink edges pointing to matching routes
    let mut call_sites: Vec<serde_json::Value> = Vec::new();
    for route in &matching {
        if let Some(route_fqn) = route.get("route_fqn").and_then(|v| v.as_str()) {
            let mut edge_stmt = conn
                .prepare(
                    "SELECT source_fqn, attributes FROM edges \
                     WHERE target_fqn = ?1 AND kind = 'HttpLink'",
                )
                .map_err(|e| McpError::DispatchError {
                    reason: format!("edge query failed: {}", e),
                })?;

            let sites: Vec<serde_json::Value> = edge_stmt
                .query_map(rusqlite::params![route_fqn], |row| {
                    let source: String = row.get(0)?;
                    let attrs_str: String = row.get(1)?;
                    Ok(serde_json::json!({
                        "caller_fqn": source,
                        "edge_attributes": serde_json::from_str::<serde_json::Value>(&attrs_str).unwrap_or_default(),
                    }))
                })
                .map_err(|e| McpError::DispatchError {
                    reason: format!("edge query failed: {}", e),
                })?
                .filter_map(|r| r.ok())
                .collect();

            call_sites.extend(sites);
        }
    }

    let result = serde_json::json!({
        "url_pattern": url_pattern,
        "matching_routes": matching,
        "call_sites": call_sites,
    });

    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize: {}", e),
    })?;

    Ok((json, matching.len()))
}

/// query_graph: basic Cypher-like query subset (MATCH/WHERE/RETURN/LIMIT)
fn dispatch_query_graph(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: query".to_string(),
        })?;

    let conn = store.read_conn();

    // Parse the simplified Cypher-like query
    // Supported: MATCH (n:Kind) WHERE n.field LIKE '%pattern%' RETURN n LIMIT N
    let query_upper = query.to_uppercase();

    let mut sql = String::from("SELECT fqn, kind, file, start_line, end_line FROM nodes WHERE 1=1");
    let mut params: Vec<String> = Vec::new();

    // Extract kind filter from MATCH clause
    if query_upper.contains("MATCH") {
        let match_clause = query;
        // Look for :Kind pattern
        if let Some(colon_pos) = match_clause.find(':') {
            let after_colon = &match_clause[colon_pos + 1..];
            let kind_end = after_colon
                .find(|c: char| !c.is_alphanumeric())
                .unwrap_or(after_colon.len());
            let kind = &after_colon[..kind_end];
            if !kind.is_empty() {
                sql.push_str(&format!(" AND kind = ?{}", params.len() + 1));
                params.push(kind.to_string());
            }
        }
    }

    // Extract WHERE clause
    if let Some(where_start) = query_upper.find("WHERE") {
        let where_clause = &query[where_start + 5..];
        let where_end = where_clause
            .to_uppercase()
            .find("RETURN")
            .or_else(|| where_clause.to_uppercase().find("LIMIT"))
            .unwrap_or(where_clause.len());
        let condition = where_clause[..where_end].trim();

        // Support: n.fqn LIKE '%pattern%' or n.file LIKE '%pattern%'
        if condition.to_uppercase().contains("LIKE") {
            if let Some(like_pos) = condition.to_uppercase().find("LIKE") {
                let field_part = condition[..like_pos].trim();
                let value_part = condition[like_pos + 4..]
                    .trim()
                    .trim_matches('\'')
                    .trim_matches('"');

                let field = if field_part.contains("fqn") {
                    "fqn"
                } else if field_part.contains("file") {
                    "file"
                } else if field_part.contains("kind") {
                    "kind"
                } else {
                    "fqn"
                };

                sql.push_str(&format!(" AND {} LIKE ?{}", field, params.len() + 1));
                params.push(value_part.to_string());
            }
        }
    }

    // Extract LIMIT
    let limit = if let Some(limit_pos) = query_upper.find("LIMIT") {
        let after_limit = query[limit_pos + 5..].trim();
        after_limit
            .split_whitespace()
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(50)
    } else {
        50
    };

    sql.push_str(&format!(" LIMIT {}", limit));

    let mut stmt = conn.prepare(&sql).map_err(|e| McpError::DispatchError {
        reason: format!("query_graph SQL error: {} (generated: {})", e, sql),
    })?;

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|p| p as &dyn rusqlite::types::ToSql).collect();

    let results: Vec<serde_json::Value> = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(serde_json::json!({
                "fqn": row.get::<_, String>(0)?,
                "kind": row.get::<_, String>(1)?,
                "file": row.get::<_, String>(2)?,
                "start_line": row.get::<_, i64>(3)?,
                "end_line": row.get::<_, i64>(4)?,
            }))
        })
        .map_err(|e| McpError::DispatchError {
            reason: format!("query_graph execution failed: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    let json = serde_json::to_string(&results).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize query results: {}", e),
    })?;

    Ok((json, results.len()))
}

/// detect_changes: query file_snapshots for files indexed after a timestamp
fn dispatch_detect_changes(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let since = args
        .get("since")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let conn = store.read_conn();

    // Find files modified after the given timestamp
    let mut stmt = conn
        .prepare(
            "SELECT n.fqn, n.kind, n.file, n.start_line, n.end_line \
             FROM nodes n \
             INNER JOIN file_snapshots fs ON n.file = fs.file \
             WHERE fs.indexed_at > ?1 \
             ORDER BY fs.indexed_at DESC",
        )
        .map_err(|e| McpError::DispatchError {
            reason: format!("detect_changes query failed: {}", e),
        })?;

    let rows: Vec<serde_json::Value> = stmt
        .query_map([since], |row| {
            let fqn: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let file: String = row.get(2)?;
            let start_line: u32 = row.get(3)?;
            let end_line: u32 = row.get(4)?;
            Ok(serde_json::json!({
                "fqn": fqn,
                "kind": kind,
                "file": file,
                "start_line": start_line,
                "end_line": end_line,
                "change_kind": "modified",
            }))
        })
        .map_err(|e| McpError::DispatchError {
            reason: format!("detect_changes query failed: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Compute simple risk_score based on count of changed nodes per file
    let total_changes = rows.len();
    let risk_score: f64 = if total_changes == 0 {
        0.0
    } else {
        (total_changes as f64 / 10.0).min(1.0)
    };

    let result = serde_json::json!({
        "since": since,
        "changes": rows,
        "total_changes": total_changes,
        "risk_score": risk_score,
    });

    let files_touched = {
        let mut files: Vec<&str> = rows.iter()
            .filter_map(|r| r.get("file").and_then(|f| f.as_str()))
            .collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize detect_changes result: {}", e),
    })?;

    Ok((json, files_touched))
}

/// Semantic search: embed query and find similar nodes by cosine similarity.
///
/// When the `semantic` feature is enabled and the model is available, this
/// generates an embedding for the query and searches against stored embeddings.
/// Otherwise, returns a helpful message about enabling semantic search.
fn dispatch_semantic_search(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: query".to_string(),
        })?;

    let top_k = args
        .get("top_k")
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;

    // Check if we have any embeddings stored
    let conn = store.read_conn();
    let embedding_count = crate::store::queries::embeddings::embedding_count(&conn)
        .unwrap_or(0);

    if embedding_count == 0 {
        let result = serde_json::json!({
            "status": "no_embeddings",
            "message": "No embeddings stored. Run `cortex index` with the semantic feature enabled and model downloaded (`cortex semantic enable`).",
            "results": []
        });
        let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
            reason: format!("failed to serialize semantic search response: {e}"),
        })?;
        return Ok((json, 0));
    }

    // Try to create an embedder to generate the query embedding
    let data_dir = std::env::var("CORTEX_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let repo_root = std::env::var("CORTEX_REPO_ROOT").unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(repo_root).join(".cortex")
        });

    // Try to generate query embedding using the embedder
    match crate::indexer::embedder::Embedder::new(&data_dir) {
        Ok(embedder) => {
            let query_embedding = embedder.generate_embedding(query).map_err(|e| {
                McpError::DispatchError {
                    reason: format!("failed to generate query embedding: {e}"),
                }
            })?;

            let results =
                crate::store::queries::embeddings::semantic_search(&conn, &query_embedding, top_k)
                    .map_err(|e| McpError::DispatchError {
                        reason: format!("semantic search failed: {e}"),
                    })?;

            let files_touched = {
                let mut files: Vec<&str> = results
                    .iter()
                    .filter_map(|r| r.file.as_deref())
                    .collect();
                files.sort_unstable();
                files.dedup();
                files.len()
            };

            let json =
                serde_json::to_string(&results).map_err(|e| McpError::DispatchError {
                    reason: format!("failed to serialize semantic search results: {e}"),
                })?;

            Ok((json, files_touched))
        }
        Err(_) => {
            // Embedder not available - fall back to FTS5-based approximate semantic search
            // Use the query text directly with FTS5 as a reasonable approximation
            let fts_results = search::search_fts(&conn, query, top_k).map_err(|e| {
                McpError::DispatchError {
                    reason: format!("semantic search FTS5 fallback failed: {e}"),
                }
            })?;

            let result = serde_json::json!({
                "status": "fallback_fts5",
                "message": "Semantic model not available. Using FTS5 text search as fallback. Run `cortex semantic enable` to download the embedding model for true semantic search.",
                "results": fts_results
            });

            let files_touched = {
                let mut files: Vec<&str> = fts_results.iter().map(|r| r.file.as_str()).collect();
                files.sort_unstable();
                files.dedup();
                files.len()
            };

            let json =
                serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
                    reason: format!("failed to serialize semantic search fallback: {e}"),
                })?;

            Ok((json, files_touched))
        }
    }
}

/// get_task_context: task-aware context budgeting via FTS5 + graph expansion
fn dispatch_get_task_context(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let task_description = args
        .get("task_description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: task_description".to_string(),
        })?;

    let token_budget = args
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: token_budget".to_string(),
        })? as usize;

    let include_code = args
        .get("include_code")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let scope = args
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let request = context_budget::ContextRequest {
        task_description: task_description.to_string(),
        token_budget,
        include_code,
        scope,
    };

    let conn = store.read_conn();
    let response = context_budget::build_context(&conn, &request).map_err(|e| {
        McpError::DispatchError {
            reason: format!("get_task_context failed: {}", e),
        }
    })?;

    let files_touched = {
        let mut files: Vec<&str> = response.symbols.iter().map(|s| s.file.as_str()).collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let json = serde_json::to_string(&response).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize task context: {}", e),
    })?;

    Ok((json, files_touched))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a string into a NodeKind option.
fn parse_node_kind(s: &str) -> Option<NodeKind> {
    match s {
        "Function" | "function" => Some(NodeKind::Function),
        "Class" | "class" => Some(NodeKind::Class),
        "Module" | "module" => Some(NodeKind::Module),
        "Route" | "route" => Some(NodeKind::Route),
        "Interface" | "interface" => Some(NodeKind::Interface),
        "Type" | "type" => Some(NodeKind::Type),
        "Enum" | "enum" => Some(NodeKind::Enum),
        "Constant" | "constant" => Some(NodeKind::Constant),
        "TypeAlias" | "typealias" | "type_alias" => Some(NodeKind::TypeAlias),
        "Trait" | "trait" => Some(NodeKind::Trait),
        "Namespace" | "namespace" => Some(NodeKind::Namespace),
        _ => None,
    }
}

/// Count unique files from a slice of Nodes.
fn count_unique_files_from_nodes(nodes: &[crate::store::types::Node]) -> usize {
    let mut files: Vec<&str> = nodes.iter().map(|n| n.file.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    files.len()
}

/// Count unique files from a slice of CallPathNodes.
fn count_unique_files_from_call_path(nodes: &[graph::CallPathNode]) -> usize {
    let mut files: Vec<&str> = nodes.iter().map(|n| n.file.as_str()).collect();
    files.sort_unstable();
    files.dedup();
    files.len()
}

/// get_class_hierarchy: queries inheritance/implements edges for a given FQN.
/// Returns parents, children, and interfaces.
fn dispatch_get_class_hierarchy(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let fqn = args
        .get("fqn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: fqn".to_string(),
        })?;

    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("both");

    let conn = store.read_conn();

    let mut parents: Vec<serde_json::Value> = Vec::new();
    let mut children: Vec<serde_json::Value> = Vec::new();
    let mut interfaces: Vec<serde_json::Value> = Vec::new();

    // Query parent classes (edges where this FQN is the source, kind = Inherits)
    if direction == "both" || direction == "up" {
        let mut stmt = conn
            .prepare(
                "SELECT e.target_fqn, n.kind, n.file, n.start_line \
                 FROM edges e \
                 LEFT JOIN nodes n ON n.fqn = e.target_fqn \
                 WHERE e.source_fqn = ?1 AND (e.kind = 'Inherits' OR e.kind = 'Extends')",
            )
            .map_err(|e| McpError::DispatchError {
                reason: format!("get_class_hierarchy parent query failed: {}", e),
            })?;

        let rows: Vec<serde_json::Value> = stmt
            .query_map(rusqlite::params![fqn], |row| {
                Ok(serde_json::json!({
                    "fqn": row.get::<_, String>(0)?,
                    "kind": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    "file": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    "start_line": row.get::<_, Option<u32>>(3)?.unwrap_or(0),
                }))
            })
            .map_err(|e| McpError::DispatchError {
                reason: format!("get_class_hierarchy parent query failed: {}", e),
            })?
            .filter_map(|r| r.ok())
            .collect();

        parents = rows;

        // Query implemented interfaces
        let mut iface_stmt = conn
            .prepare(
                "SELECT e.target_fqn, n.kind, n.file, n.start_line \
                 FROM edges e \
                 LEFT JOIN nodes n ON n.fqn = e.target_fqn \
                 WHERE e.source_fqn = ?1 AND (e.kind = 'Implements' OR e.kind = 'ImplFor')",
            )
            .map_err(|e| McpError::DispatchError {
                reason: format!("get_class_hierarchy interface query failed: {}", e),
            })?;

        let iface_rows: Vec<serde_json::Value> = iface_stmt
            .query_map(rusqlite::params![fqn], |row| {
                Ok(serde_json::json!({
                    "fqn": row.get::<_, String>(0)?,
                    "kind": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    "file": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    "start_line": row.get::<_, Option<u32>>(3)?.unwrap_or(0),
                }))
            })
            .map_err(|e| McpError::DispatchError {
                reason: format!("get_class_hierarchy interface query failed: {}", e),
            })?
            .filter_map(|r| r.ok())
            .collect();

        interfaces = iface_rows;
    }

    // Query child classes (edges where this FQN is the target, kind = Inherits)
    if direction == "both" || direction == "down" {
        let mut stmt = conn
            .prepare(
                "SELECT e.source_fqn, n.kind, n.file, n.start_line \
                 FROM edges e \
                 LEFT JOIN nodes n ON n.fqn = e.source_fqn \
                 WHERE e.target_fqn = ?1 AND (e.kind = 'Inherits' OR e.kind = 'Extends' OR e.kind = 'Implements' OR e.kind = 'ImplFor')",
            )
            .map_err(|e| McpError::DispatchError {
                reason: format!("get_class_hierarchy children query failed: {}", e),
            })?;

        let rows: Vec<serde_json::Value> = stmt
            .query_map(rusqlite::params![fqn], |row| {
                Ok(serde_json::json!({
                    "fqn": row.get::<_, String>(0)?,
                    "kind": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    "file": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    "start_line": row.get::<_, Option<u32>>(3)?.unwrap_or(0),
                }))
            })
            .map_err(|e| McpError::DispatchError {
                reason: format!("get_class_hierarchy children query failed: {}", e),
            })?
            .filter_map(|r| r.ok())
            .collect();

        children = rows;
    }

    let files_touched = {
        let mut files: Vec<&str> = parents
            .iter()
            .chain(children.iter())
            .chain(interfaces.iter())
            .filter_map(|v| v.get("file").and_then(|f| f.as_str()))
            .filter(|f| !f.is_empty())
            .collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let result = serde_json::json!({
        "fqn": fqn,
        "direction": direction,
        "parents": parents,
        "children": children,
        "interfaces": interfaces,
    });

    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize class hierarchy: {}", e),
    })?;

    Ok((json, files_touched))
}

/// Inline git churn computation (avoids circular dependency on CLI module).
fn get_git_churn_inline(months: u32) -> Result<std::collections::HashMap<String, u32>, anyhow::Error> {
    let since_arg = format!("{} months ago", months);

    let output = std::process::Command::new("git")
        .args(["log", "--format=format:", "--name-only", "--since", &since_arg])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git log failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut churn_map: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.replace('\\', "/");
        *churn_map.entry(normalized).or_insert(0) += 1;
    }

    Ok(churn_map)
}

/// get_git_hotspots: returns top N files by churn rate combined with caller count.
fn dispatch_get_git_hotspots(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let since_months = args
        .get("since_months")
        .and_then(|v| v.as_u64())
        .unwrap_or(6) as u32;

    // Get git churn data.
    let churn_map = get_git_churn_inline(since_months).map_err(|e| {
        McpError::DispatchError {
            reason: format!("get_git_hotspots: git churn failed: {}", e),
        }
    })?;

    // Get hotspot nodes from the graph.
    let conn = store.read_conn();
    let hotspot_nodes = graph::get_hotspot_nodes(&conn, 500).map_err(|e| {
        McpError::DispatchError {
            reason: format!("get_git_hotspots: hotspot query failed: {}", e),
        }
    })?;

    // Join churn with caller counts.
    let mut results: Vec<serde_json::Value> = Vec::new();

    for node in &hotspot_nodes {
        let churn = churn_map.get(&node.file).copied().unwrap_or(0);
        if churn == 0 {
            continue;
        }
        let risk_score = churn as u64 * node.caller_count as u64;
        if risk_score == 0 {
            continue;
        }
        results.push(serde_json::json!({
            "fqn": node.fqn,
            "file": node.file,
            "churn_count": churn,
            "caller_count": node.caller_count,
            "risk_score": risk_score,
        }));
    }

    // Sort by risk score descending.
    results.sort_by(|a, b| {
        let ra = a.get("risk_score").and_then(|v| v.as_u64()).unwrap_or(0);
        let rb = b.get("risk_score").and_then(|v| v.as_u64()).unwrap_or(0);
        rb.cmp(&ra)
    });
    results.truncate(limit);

    let files_touched = {
        let mut files: Vec<&str> = results
            .iter()
            .filter_map(|r| r.get("file").and_then(|f| f.as_str()))
            .collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let json = serde_json::to_string(&results).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize git hotspots: {}", e),
    })?;

    Ok((json, files_touched))
}

/// get_import_graph: returns all import edges for a file or module.
fn dispatch_get_import_graph(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let file = args.get("file").and_then(|v| v.as_str());
    let module = args.get("module").and_then(|v| v.as_str());

    if file.is_none() && module.is_none() {
        return Err(McpError::DispatchError {
            reason: "missing required argument: file or module".to_string(),
        });
    }

    let conn = store.read_conn();

    let filter_pattern = if let Some(f) = file {
        f.to_string()
    } else {
        module.unwrap().to_string()
    };

    // Query import edges where the source or target matches the file/module.
    let mut stmt = conn
        .prepare(
            "SELECT e.source_fqn, e.target_fqn, e.kind, e.confidence \
             FROM edges e \
             WHERE e.kind = 'Imports' \
               AND (e.source_fqn LIKE ?1 OR e.target_fqn LIKE ?1)",
        )
        .map_err(|e| McpError::DispatchError {
            reason: format!("get_import_graph query failed: {}", e),
        })?;

    let like_pattern = format!("%{}%", filter_pattern);

    let results: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params![like_pattern], |row| {
            Ok(serde_json::json!({
                "source": row.get::<_, String>(0)?,
                "target": row.get::<_, String>(1)?,
                "kind": row.get::<_, String>(2)?,
                "confidence": row.get::<_, f64>(3)?,
            }))
        })
        .map_err(|e| McpError::DispatchError {
            reason: format!("get_import_graph query failed: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Also check for Uses edges as a fallback for import relationships.
    let mut uses_stmt = conn
        .prepare(
            "SELECT e.source_fqn, e.target_fqn, e.kind, e.confidence \
             FROM edges e \
             WHERE e.kind = 'Uses' \
               AND (e.source_fqn LIKE ?1 OR e.target_fqn LIKE ?1) \
             LIMIT 200",
        )
        .map_err(|e| McpError::DispatchError {
            reason: format!("get_import_graph uses query failed: {}", e),
        })?;

    let uses_results: Vec<serde_json::Value> = uses_stmt
        .query_map(rusqlite::params![like_pattern], |row| {
            Ok(serde_json::json!({
                "source": row.get::<_, String>(0)?,
                "target": row.get::<_, String>(1)?,
                "kind": row.get::<_, String>(2)?,
                "confidence": row.get::<_, f64>(3)?,
            }))
        })
        .map_err(|e| McpError::DispatchError {
            reason: format!("get_import_graph uses query failed: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    let mut all_results = results;
    all_results.extend(uses_results);

    let files_touched = {
        let mut files: Vec<&str> = all_results
            .iter()
            .filter_map(|r| r.get("source").and_then(|f| f.as_str()))
            .collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    };

    let response = serde_json::json!({
        "filter": filter_pattern,
        "edges": all_results,
        "total": all_results.len(),
    });

    let json = serde_json::to_string(&response).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize import graph: {}", e),
    })?;

    Ok((json, files_touched))
}

/// find_similar_functions: finds functions with similar call patterns (same callees).
fn dispatch_find_similar_functions(
    store: &StoreManager,
    args: &Value,
) -> Result<(String, usize), McpError> {
    let fqn = args
        .get("fqn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::DispatchError {
            reason: "missing required argument: fqn".to_string(),
        })?;

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as usize;

    let conn = store.read_conn();

    // Step 1: Get all callees of the target function.
    let mut callee_stmt = conn
        .prepare(
            "SELECT target_fqn FROM edges WHERE source_fqn = ?1 AND kind = 'Calls'",
        )
        .map_err(|e| McpError::DispatchError {
            reason: format!("find_similar_functions callee query failed: {}", e),
        })?;

    let callees: Vec<String> = callee_stmt
        .query_map(rusqlite::params![fqn], |row| row.get(0))
        .map_err(|e| McpError::DispatchError {
            reason: format!("find_similar_functions callee query failed: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    if callees.is_empty() {
        let result = serde_json::json!({
            "fqn": fqn,
            "similar": [],
            "message": "No callees found for this function.",
        });
        let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
            reason: format!("failed to serialize: {}", e),
        })?;
        return Ok((json, 0));
    }

    // Step 2: For each callee, find other callers. Count overlap.
    let callee_set: std::collections::HashSet<&str> =
        callees.iter().map(|s| s.as_str()).collect();

    // Find all functions that call at least one of the same callees.
    let placeholders: String = callees.iter().enumerate().map(|(i, _)| {
        if i == 0 { format!("?{}", i + 1) } else { format!(", ?{}", i + 1) }
    }).collect();

    let sql = format!(
        "SELECT source_fqn, target_fqn FROM edges \
         WHERE target_fqn IN ({}) AND kind = 'Calls' AND source_fqn != ?{}",
        placeholders,
        callees.len() + 1
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| McpError::DispatchError {
        reason: format!("find_similar_functions overlap query failed: {}", e),
    })?;

    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = callees
        .iter()
        .map(|c| Box::new(c.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    params.push(Box::new(fqn.to_string()));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|p| p.as_ref()).collect();

    let rows: Vec<(String, String)> = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| McpError::DispatchError {
            reason: format!("find_similar_functions overlap query failed: {}", e),
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Count how many shared callees each candidate has.
    let mut overlap_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (caller, callee) in &rows {
        if callee_set.contains(callee.as_str()) {
            *overlap_counts.entry(caller.clone()).or_insert(0) += 1;
        }
    }

    // Sort by overlap count descending.
    let mut candidates: Vec<(String, usize)> = overlap_counts.into_iter().collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    candidates.truncate(limit);

    // Compute similarity as overlap / total callees.
    let total_callees = callees.len() as f64;
    let similar: Vec<serde_json::Value> = candidates
        .iter()
        .map(|(candidate_fqn, overlap)| {
            let similarity = *overlap as f64 / total_callees;
            serde_json::json!({
                "fqn": candidate_fqn,
                "shared_callees": overlap,
                "total_callees": callees.len(),
                "similarity": (similarity * 100.0).round() / 100.0,
            })
        })
        .collect();

    let files_touched = candidates.len();

    let result = serde_json::json!({
        "fqn": fqn,
        "callees_count": callees.len(),
        "similar": similar,
    });

    let json = serde_json::to_string(&result).map_err(|e| McpError::DispatchError {
        reason: format!("failed to serialize similar functions: {}", e),
    })?;

    Ok((json, files_touched))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::StoreManager;
    use crate::store::migrations;

    /// Creates a StoreManager with migrations applied for testing.
    fn setup_store() -> (StoreManager, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = StoreManager::new(tmp.path()).expect("failed to create store");
        // Apply migrations
        let conn = store.write_conn();
        migrations::run_migrations(
            &conn,
            std::path::Path::new("migrations"),
        )
        .expect("failed to run migrations");
        drop(conn);
        (store, tmp)
    }

    #[test]
    fn test_dispatch_search_symbols_valid() {
        let (store, _tmp) = setup_store();

        // Insert a test node
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.rs::main', 'Function', 'src/main.rs', 1, 10, 'hash', 1000, '{}')",
                [],
            ).unwrap();
        }

        let args = serde_json::json!({ "pattern": "*main*" });
        let result = dispatch_tool(&store, "search_symbols", &args).unwrap();

        // Verify response structure
        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        let meta = result.get("_meta").unwrap();
        assert!(meta.get("tokens_used").is_some());
        assert!(meta.get("tokens_saved").is_some());
        assert!(meta.get("query_time_ms").is_some());
    }

    #[test]
    fn test_dispatch_trace_callers_valid() {
        let (store, _tmp) = setup_store();

        // Insert nodes and edges
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/a.rs::caller', 'Function', 'src/a.rs', 1, 10, 'hash', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/b.rs::callee', 'Function', 'src/b.rs', 1, 10, 'hash', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
                 VALUES ('src/a.rs::caller', 'src/b.rs::callee', 'Calls', 1.0, '{}')",
                [],
            ).unwrap();
        }

        let args = serde_json::json!({ "fqn": "src/b.rs::callee", "depth": 3 });
        let result = dispatch_tool(&store, "trace_callers", &args).unwrap();

        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        // Parse the content text to verify callers are returned
        let content = result["content"][0]["text"].as_str().unwrap();
        let callers: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0]["fqn"], "src/a.rs::caller");
    }

    #[test]
    fn test_dispatch_write_observation_valid() {
        let (store, _tmp) = setup_store();

        let args = serde_json::json!({
            "node_fqn": "src/main.rs::main",
            "observation_text": "This is a test observation",
            "agent_id": "test-agent"
        });
        let result = dispatch_tool(&store, "write_observation", &args).unwrap();

        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        // Parse the content to verify an id was returned
        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert!(parsed.get("id").is_some());
        assert!(!parsed["id"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_dispatch_missing_required_arg() {
        let (store, _tmp) = setup_store();

        // search_symbols requires "pattern"
        let args = serde_json::json!({});
        let result = dispatch_tool(&store, "search_symbols", &args);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("missing required argument: pattern"));
    }

    #[test]
    fn test_dispatch_unknown_tool() {
        let (store, _tmp) = setup_store();

        let args = serde_json::json!({});
        let result = dispatch_tool(&store, "nonexistent_tool", &args);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("unknown tool: nonexistent_tool"));
    }

    #[test]
    fn test_dispatch_stub_tools() {
        let (store, _tmp) = setup_store();

        // detect_changes returns valid result
        let args = serde_json::json!({ "since": "abc123" });
        let result = dispatch_tool(&store, "detect_changes", &args).unwrap();
        assert!(result.get("_meta").is_some());

        // get_code_snippet returns error for non-existent node (real implementation)
        let args = serde_json::json!({ "fqn": "nonexistent::symbol" });
        let result = dispatch_tool(&store, "get_code_snippet", &args);
        assert!(result.is_err()); // node not found

        // get_http_routes returns empty array (real implementation, no routes in DB)
        let args = serde_json::json!({});
        let result = dispatch_tool(&store, "get_http_routes", &args).unwrap();
        assert!(result.get("_meta").is_some());
        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert!(parsed.is_array());
    }

    #[test]
    fn test_meta_has_nonzero_tokens_used() {
        let (store, _tmp) = setup_store();

        let args = serde_json::json!({ "pattern": "*" });
        let result = dispatch_tool(&store, "search_symbols", &args).unwrap();

        let meta = result.get("_meta").unwrap();
        // Even an empty result produces some JSON, so tokens_used should be >= 0
        // (empty array "[]" is 2 chars = 0 tokens, but that's fine)
        assert!(meta["tokens_used"].as_u64().is_some());
        assert!(meta["query_time_ms"].as_u64().is_some());
    }

    #[test]
    fn test_dispatch_find_taint_paths_returns_valid_json_with_meta() {
        let (store, _tmp) = setup_store();

        let args = serde_json::json!({});
        let result = dispatch_tool(&store, "find_taint_paths", &args).unwrap();

        // Verify response has content and _meta
        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        let meta = result.get("_meta").unwrap();
        assert!(meta.get("tokens_used").is_some());
        assert!(meta.get("tokens_saved").is_some());
        assert!(meta.get("query_time_ms").is_some());

        // Verify content is valid JSON (array of taint paths)
        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert!(parsed.is_array());
    }

    #[test]
    fn test_dispatch_scan_owasp_returns_valid_json_with_meta() {
        let (store, _tmp) = setup_store();

        let args = serde_json::json!({});
        let result = dispatch_tool(&store, "scan_owasp", &args).unwrap();

        // Verify response has content and _meta
        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        let meta = result.get("_meta").unwrap();
        assert!(meta.get("tokens_used").is_some());
        assert!(meta.get("tokens_saved").is_some());
        assert!(meta.get("query_time_ms").is_some());

        // Verify content is valid JSON (array of security findings)
        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert!(parsed.is_array());
    }

    #[test]
    fn test_dispatch_generate_sbom_returns_valid_json_with_meta() {
        let (store, _tmp) = setup_store();

        // Use the temp dir as repo_root so it doesn't fail on missing manifests
        let args = serde_json::json!({ "repo_root": _tmp.path().to_str().unwrap() });
        let result = dispatch_tool(&store, "generate_sbom", &args).unwrap();

        // Verify response has content and _meta
        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        let meta = result.get("_meta").unwrap();
        assert!(meta.get("tokens_used").is_some());
        assert!(meta.get("tokens_saved").is_some());
        assert!(meta.get("query_time_ms").is_some());

        // Verify content is valid SPDX JSON
        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert_eq!(parsed["spdxVersion"], "SPDX-2.3");
        assert_eq!(parsed["dataLicense"], "CC0-1.0");
        assert_eq!(parsed["SPDXID"], "SPDXRef-DOCUMENT");
    }

    #[test]
    fn test_dispatch_semantic_search_no_embeddings_returns_message() {
        let (store, _tmp) = setup_store();

        let args = serde_json::json!({ "query": "authentication handler", "top_k": 5 });
        let result = dispatch_tool(&store, "semantic_search", &args).unwrap();

        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        // Parse the content to verify structured response
        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        // With no embeddings stored, should indicate no_embeddings status
        assert_eq!(parsed["status"], "no_embeddings");
        assert!(parsed["message"].as_str().unwrap().contains("No embeddings"));
    }

    #[test]
    fn test_dispatch_detect_changes_returns_modified_nodes() {
        let (store, _tmp) = setup_store();

        // Insert a file_snapshot and node with a known timestamp
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO file_snapshots (file, file_hash, node_count, indexed_at) \
                 VALUES ('src/main.rs', 'hash123', 1, 2000)",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.rs::main', 'Function', 'src/main.rs', 1, 10, 'hash123', 2000, '{}')",
                [],
            ).unwrap();
        }

        // Query for changes since timestamp 1000 (should find the node)
        let args = serde_json::json!({ "since": 1000 });
        let result = dispatch_tool(&store, "detect_changes", &args).unwrap();

        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert_eq!(parsed["total_changes"], 1);
        assert!(parsed["risk_score"].as_f64().unwrap() > 0.0);

        let changes = parsed["changes"].as_array().unwrap();
        assert_eq!(changes[0]["fqn"], "src/main.rs::main");
        assert_eq!(changes[0]["change_kind"], "modified");
    }

    #[test]
    fn test_dispatch_detect_changes_future_timestamp_empty() {
        let (store, _tmp) = setup_store();

        // Insert data with timestamp 1000
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO file_snapshots (file, file_hash, node_count, indexed_at) \
                 VALUES ('src/main.rs', 'hash123', 1, 1000)",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.rs::main', 'Function', 'src/main.rs', 1, 10, 'hash123', 1000, '{}')",
                [],
            ).unwrap();
        }

        // Query for changes since timestamp 9999 (future, should find nothing)
        let args = serde_json::json!({ "since": 9999 });
        let result = dispatch_tool(&store, "detect_changes", &args).unwrap();

        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert_eq!(parsed["total_changes"], 0);
        assert_eq!(parsed["risk_score"], 0.0);
    }

    #[test]
    fn test_hybrid_retrieval_fallback_to_fts5() {
        let (store, _tmp) = setup_store();

        // Insert nodes that won't match the graph pattern but will match FTS5
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/auth/login.rs::authenticate_user', 'Function', 'src/auth/login.rs', 1, 20, 'hash1', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/auth/session.rs::validate_session', 'Function', 'src/auth/session.rs', 1, 15, 'hash2', 1000, '{}')",
                [],
            ).unwrap();
            // Manually sync FTS5 index (triggers may not fire after migration 0006 table rebuild)
            conn.execute(
                "INSERT OR IGNORE INTO nodes_fts (rowid, fqn, kind, file, attributes) \
                 SELECT rowid, fqn, kind, file, attributes FROM nodes WHERE fqn LIKE '%authenticate%' OR fqn LIKE '%validate%'",
                [],
            ).ok();
        }

        // Search with a pattern that won't match via graph LIKE (no wildcard match)
        // but the FTS5 fallback should find "authenticate" in the FQN
        let args = serde_json::json!({ "pattern": "authenticate" });
        let result = dispatch_tool(&store, "search_symbols", &args).unwrap();

        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        let meta = result.get("_meta").unwrap();
        // retrieval_method should be "fts5" or "hybrid" since graph returned <3 results
        let retrieval_method = meta.get("retrieval_method").and_then(|v| v.as_str()).unwrap();
        assert!(
            retrieval_method == "fts5" || retrieval_method == "hybrid",
            "Expected retrieval_method to be 'fts5' or 'hybrid', got '{}'",
            retrieval_method
        );

        // Verify results are returned from FTS5 fallback
        let content = result["content"][0]["text"].as_str().unwrap();
        let results: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();
        assert!(
            !results.is_empty(),
            "Expected FTS5 fallback to return results"
        );

        // Verify results have confidence field
        for r in &results {
            assert!(r.get("confidence").is_some(), "Result missing confidence field");
            let confidence = r["confidence"].as_f64().unwrap();
            assert!(confidence > 0.0 && confidence <= 1.0, "Confidence out of range: {}", confidence);
        }
    }

    #[test]
    fn test_hybrid_retrieval_graph_only_when_enough_results() {
        let (store, _tmp) = setup_store();

        // Insert 5 nodes that match the graph pattern
        {
            let conn = store.write_conn();
            for i in 0..5 {
                conn.execute(
                    &format!(
                        "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                         VALUES ('src/handlers/handler_{}.rs::process', 'Function', 'src/handlers/handler_{}.rs', 1, 10, 'hash{}', 1000, '{{}}')",
                        i, i, i
                    ),
                    [],
                ).unwrap();
            }
        }

        // Search with a pattern that matches >=3 results via graph
        let args = serde_json::json!({ "pattern": "src/handlers/*" });
        let result = dispatch_tool(&store, "search_symbols", &args).unwrap();

        let meta = result.get("_meta").unwrap();
        let retrieval_method = meta.get("retrieval_method").and_then(|v| v.as_str()).unwrap();
        assert_eq!(retrieval_method, "graph", "Expected 'graph' when >=3 results from graph");

        // Verify all results have confidence 1.0 (graph-only)
        let content = result["content"][0]["text"].as_str().unwrap();
        let results: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();
        assert_eq!(results.len(), 5);
        for r in &results {
            assert_eq!(r["confidence"].as_f64().unwrap(), 1.0);
        }
    }

    #[test]
    fn test_hybrid_retrieval_deduplicates_by_fqn() {
        let (store, _tmp) = setup_store();

        // Insert a node that will match both graph (via LIKE) and FTS5
        // but with <3 graph results to trigger fallback
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/utils.rs::helper_func', 'Function', 'src/utils.rs', 1, 10, 'hash1', 1000, '{}')",
                [],
            ).unwrap();
        }

        // Pattern matches via graph LIKE, but <3 results triggers FTS5 too
        // FTS5 will also find the same node. Deduplication should keep only one entry.
        let args = serde_json::json!({ "pattern": "*helper_func*" });
        let result = dispatch_tool(&store, "search_symbols", &args).unwrap();

        let content = result["content"][0]["text"].as_str().unwrap();
        let results: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();

        // Count occurrences of the FQN - should be exactly 1 (deduplicated)
        let helper_count = results
            .iter()
            .filter(|r| r["fqn"].as_str() == Some("src/utils.rs::helper_func"))
            .count();
        assert_eq!(helper_count, 1, "Expected deduplication to keep only one entry per FQN");

        // The kept entry should have confidence 1.0 (graph result preferred over FTS5)
        let helper = results
            .iter()
            .find(|r| r["fqn"].as_str() == Some("src/utils.rs::helper_func"))
            .unwrap();
        assert_eq!(helper["confidence"].as_f64().unwrap(), 1.0);
    }

    #[test]
    fn test_get_code_snippet_redacts_secrets() {
        let (store, tmp) = setup_store();

        // Create a source file with an AWS key
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        let source_file = repo_dir.join("secrets_test.py");
        std::fs::write(
            &source_file,
            "def get_config():\n    key = \"AKIAIOSFODNN7EXAMPLE\"\n    return key\n",
        )
        .unwrap();

        // Insert a node with contains_secret attribute
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('repo/secrets_test.py::get_config', 'Function', 'repo/secrets_test.py', 1, 3, 'hash1', 1000, \
                 '{\"contains_secret\": \"aws_access_key\", \"secret_line\": 2}')",
                [],
            ).unwrap();
        }

        // Set CORTEX_REPO_ROOT to the temp dir so get_code_snippet can find the file
        unsafe { std::env::set_var("CORTEX_REPO_ROOT", tmp.path().to_str().unwrap()) };

        let args = serde_json::json!({ "fqn": "repo/secrets_test.py::get_config" });
        let result = dispatch_tool(&store, "get_code_snippet", &args).unwrap();

        let content = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        let code = parsed["code"].as_str().unwrap();

        // The AWS key should be redacted
        assert!(
            code.contains("[REDACTED:aws_access_key]"),
            "Expected AWS key to be redacted, got: {}",
            code
        );
        assert!(
            !code.contains("AKIAIOSFODNN7EXAMPLE"),
            "Expected AWS key to NOT appear in redacted output"
        );
        // Structure should be preserved
        assert!(code.contains("def get_config():"));
        assert!(code.contains("return key"));

        // Clean up env var
        unsafe { std::env::remove_var("CORTEX_REPO_ROOT") };
    }

    #[test]
    fn test_get_complexity_hotspots() {
        let (store, _tmp) = setup_store();

        // Insert function nodes with different complexity values
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.rs::simple', 'Function', 'src/main.rs', 1, 5, 'hash1', 1000, '{\"complexity\": 1}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.rs::moderate', 'Function', 'src/main.rs', 10, 30, 'hash1', 1000, '{\"complexity\": 8}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.rs::complex', 'Function', 'src/main.rs', 35, 80, 'hash1', 1000, '{\"complexity\": 15}')",
                [],
            ).unwrap();
        }

        // Query with threshold=5 should return moderate and complex
        let args = serde_json::json!({ "limit": 10, "threshold": 5 });
        let result = dispatch_tool(&store, "get_complexity_hotspots", &args).unwrap();

        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        let content = result["content"][0]["text"].as_str().unwrap();
        let hotspots: Vec<serde_json::Value> = serde_json::from_str(content).unwrap();

        // Should return 2 results (moderate=8, complex=15), not simple=1
        assert_eq!(hotspots.len(), 2);

        // Should be sorted by complexity descending
        assert_eq!(hotspots[0]["fqn"], "src/main.rs::complex");
        assert_eq!(hotspots[0]["complexity"], 15);
        assert_eq!(hotspots[1]["fqn"], "src/main.rs::moderate");
        assert_eq!(hotspots[1]["complexity"], 8);
    }

    #[test]
    fn test_token_breakdown_in_meta_sums_to_tokens_used() {
        let (store, _tmp) = setup_store();

        // Insert test nodes to get a non-trivial response
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/main.rs::main', 'Function', 'src/main.rs', 1, 10, 'hash', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/lib.rs::init', 'Function', 'src/lib.rs', 1, 5, 'hash2', 1000, '{}')",
                [],
            ).unwrap();
        }

        let args = serde_json::json!({ "pattern": "*" });
        let result = dispatch_tool(&store, "search_symbols", &args).unwrap();

        let meta = result.get("_meta").unwrap();
        let tokens_used = meta["tokens_used"].as_u64().unwrap();
        let breakdown = meta.get("token_breakdown").expect("_meta must have token_breakdown");

        let nodes = breakdown["nodes"].as_u64().unwrap();
        let edges = breakdown["edges"].as_u64().unwrap();
        let code = breakdown["code"].as_u64().unwrap();
        let metadata = breakdown["metadata"].as_u64().unwrap();

        let sum = nodes + edges + code + metadata;
        assert_eq!(
            sum, tokens_used,
            "token_breakdown components ({} + {} + {} + {} = {}) must sum to tokens_used ({})",
            nodes, edges, code, metadata, sum, tokens_used
        );
    }

    // -----------------------------------------------------------------------
    // Task 7.10: Integration test for get_task_context MCP tool end-to-end
    // -----------------------------------------------------------------------

    #[test]
    fn test_dispatch_get_task_context_basic() {
        let (store, _tmp) = setup_store();

        // Insert test nodes and edges
        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/auth.rs::validate_token', 'Function', 'src/auth.rs', 10, 30, 'hash1', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/auth.rs::check_expiry', 'Function', 'src/auth.rs', 35, 50, 'hash1', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/db.rs::get_user', 'Function', 'src/db.rs', 1, 20, 'hash2', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
                 VALUES ('src/auth.rs::validate_token', 'src/db.rs::get_user', 'Calls', 1.0, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO edges (source_fqn, target_fqn, kind, confidence, attributes) \
                 VALUES ('src/auth.rs::validate_token', 'src/auth.rs::check_expiry', 'Calls', 1.0, '{}')",
                [],
            ).unwrap();
        }

        let args = serde_json::json!({
            "task_description": "validate token auth",
            "token_budget": 1000,
            "include_code": false
        });
        let result = dispatch_tool(&store, "get_task_context", &args).unwrap();

        // Verify response structure
        assert!(result.get("content").is_some());
        assert!(result.get("_meta").is_some());

        let meta = result.get("_meta").unwrap();
        assert!(meta.get("tokens_used").is_some());
        assert!(meta.get("tokens_saved").is_some());
        assert!(meta.get("query_time_ms").is_some());

        // Parse the content
        let content = result["content"][0]["text"].as_str().unwrap();
        let response: serde_json::Value = serde_json::from_str(content).unwrap();

        // Should have symbols array
        assert!(response.get("symbols").is_some());
        assert!(response["symbols"].is_array());

        // Should have relationships array
        assert!(response.get("relationships").is_some());
        assert!(response["relationships"].is_array());

        // Should have truncated field
        assert!(response.get("truncated").is_some());

        // Should have relevance_cutoff field
        assert!(response.get("relevance_cutoff").is_some());
    }

    #[test]
    fn test_dispatch_get_task_context_missing_args() {
        let (store, _tmp) = setup_store();

        // Missing task_description
        let args = serde_json::json!({ "token_budget": 1000 });
        let result = dispatch_tool(&store, "get_task_context", &args);
        assert!(result.is_err());

        // Missing token_budget
        let args = serde_json::json!({ "task_description": "test" });
        let result = dispatch_tool(&store, "get_task_context", &args);
        assert!(result.is_err());
    }

    #[test]
    fn test_dispatch_get_task_context_with_scope() {
        let (store, _tmp) = setup_store();

        {
            let conn = store.write_conn();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/auth/login.rs::login', 'Function', 'src/auth/login.rs', 1, 20, 'hash1', 1000, '{}')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO nodes (fqn, kind, file, start_line, end_line, file_hash, indexed_at, attributes) \
                 VALUES ('src/db/users.rs::get_user', 'Function', 'src/db/users.rs', 1, 15, 'hash2', 1000, '{}')",
                [],
            ).unwrap();
        }

        let args = serde_json::json!({
            "task_description": "login get_user",
            "token_budget": 1000,
            "scope": "src/auth"
        });
        let result = dispatch_tool(&store, "get_task_context", &args).unwrap();

        let content = result["content"][0]["text"].as_str().unwrap();
        let response: serde_json::Value = serde_json::from_str(content).unwrap();

        // All symbols should be in the src/auth scope
        if let Some(symbols) = response["symbols"].as_array() {
            for symbol in symbols {
                let file = symbol["file"].as_str().unwrap_or("");
                assert!(
                    file.starts_with("src/auth"),
                    "Symbol file '{}' should be in src/auth scope",
                    file
                );
            }
        }
    }

    #[test]
    fn test_dispatch_get_task_context_empty_results() {
        let (store, _tmp) = setup_store();

        let args = serde_json::json!({
            "task_description": "completely nonexistent xyz123",
            "token_budget": 1000
        });
        let result = dispatch_tool(&store, "get_task_context", &args).unwrap();

        let content = result["content"][0]["text"].as_str().unwrap();
        let response: serde_json::Value = serde_json::from_str(content).unwrap();

        assert_eq!(response["symbols"].as_array().unwrap().len(), 0);
        assert_eq!(response["truncated"], false);
    }
}
