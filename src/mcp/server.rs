//! MCP server implementation with JSON-RPC 2.0 over stdio.
//!
//! Reads newline-delimited JSON from stdin, dispatches to method handlers,
//! and writes newline-delimited JSON responses to stdout. Concurrent tool
//! executions are limited to 4 via a semaphore.

use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

use crate::error::McpError;
use crate::store::db::StoreManager;

use super::types::{
    InitializeResult, JsonRpcRequest, JsonRpcResponse, ServerCapabilities, ServerInfo,
    ToolDefinition, ToolsCapability, ToolsListResult,
    INTERNAL_ERROR, METHOD_NOT_FOUND, PARSE_ERROR,
};

/// Maximum number of concurrent tool executions.
const MAX_CONCURRENT_TOOL_CALLS: usize = 4;

/// MCP protocol version supported by this server.
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Server name reported during initialization.
const SERVER_NAME: &str = "cortex";

/// Server version reported during initialization.
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The MCP server that handles JSON-RPC 2.0 requests over stdio.
pub struct McpServer {
    store: Arc<StoreManager>,
    semaphore: Arc<Semaphore>,
    smart_tools: bool,
}

impl McpServer {
    /// Creates a new MCP server with the given store manager.
    pub fn new(store: Arc<StoreManager>) -> Self {
        Self {
            store,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_TOOL_CALLS)),
            smart_tools: false,
        }
    }

    /// Creates a new MCP server with smart-tools mode enabled.
    /// In smart-tools mode, only 5 core tools are exposed to reduce context overhead.
    pub fn with_smart_tools(store: Arc<StoreManager>, smart_tools: bool) -> Self {
        Self {
            store,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_TOOL_CALLS)),
            smart_tools,
        }
    }

    /// Runs the MCP server event loop, reading from stdin and writing to stdout.
    pub async fn run(self) -> Result<(), McpError> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        info!("MCP server started, listening on stdin");

        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }

            let response = self.handle_line(&line).await;

            // Notifications (no id) do not get a response
            if let Some(response) = response {
                let json = serde_json::to_string(&response).map_err(|e| {
                    McpError::ProtocolError {
                        reason: format!("failed to serialize response: {}", e),
                    }
                })?;

                stdout
                    .write_all(json.as_bytes())
                    .await
                    .map_err(|e| McpError::ProtocolError {
                        reason: format!("failed to write to stdout: {}", e),
                    })?;
                stdout
                    .write_all(b"\n")
                    .await
                    .map_err(|e| McpError::ProtocolError {
                        reason: format!("failed to write newline to stdout: {}", e),
                    })?;
                stdout.flush().await.map_err(|e| McpError::ProtocolError {
                    reason: format!("failed to flush stdout: {}", e),
                })?;
            }
        }

        info!("MCP server shutting down (stdin closed)");
        Ok(())
    }

    /// Handles a single line of input, returning an optional response.
    /// Returns None for notifications (requests without an id).
    pub async fn handle_line(&self, line: &str) -> Option<JsonRpcResponse> {
        // Parse the JSON
        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(e) => {
                warn!("Failed to parse JSON-RPC request: {}", e);
                return Some(JsonRpcResponse::error(
                    None,
                    PARSE_ERROR,
                    format!("Parse error: {}", e),
                ));
            }
        };

        debug!("Received request: method={}", request.method);

        // Handle the request
        self.handle_request(request).await
    }

    /// Processes a parsed JSON-RPC request and returns an optional response.
    pub async fn handle_request(&self, request: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = request.id.clone();
        let method = request.method.as_str();

        // Notifications (no id) that we acknowledge silently
        if id.is_none() {
            match method {
                "notifications/initialized" => {
                    debug!("Received initialized notification");
                    return None;
                }
                _ => {
                    debug!("Received unknown notification: {}", method);
                    return None;
                }
            }
        }

        // Methods that require a response
        let result = match method {
            "initialize" => self.handle_initialize(&request),
            "tools/list" => self.handle_tools_list(),
            "tools/call" => self.handle_tools_call(&request).await,
            _ => {
                warn!("Unknown method: {}", method);
                return Some(JsonRpcResponse::error(
                    id,
                    METHOD_NOT_FOUND,
                    format!("Method not found: {}", method),
                ));
            }
        };

        match result {
            Ok(value) => Some(JsonRpcResponse::success(id, value)),
            Err(e) => Some(JsonRpcResponse::error(
                id,
                INTERNAL_ERROR,
                format!("Internal error: {}", e),
            )),
        }
    }

    /// Handles the `initialize` method, returning server capabilities.
    fn handle_initialize(&self, _request: &JsonRpcRequest) -> Result<Value, McpError> {
        let result = InitializeResult {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: ServerCapabilities {
                tools: ToolsCapability {
                    list_changed: false,
                },
            },
            server_info: ServerInfo {
                name: SERVER_NAME.to_string(),
                version: SERVER_VERSION.to_string(),
            },
        };

        serde_json::to_value(result).map_err(|e| McpError::ProtocolError {
            reason: format!("failed to serialize initialize result: {}", e),
        })
    }

    /// Handles the `tools/list` method, returning all available tool definitions.
    fn handle_tools_list(&self) -> Result<Value, McpError> {
        let tools = if self.smart_tools {
            get_smart_tool_definitions()
        } else {
            get_tool_definitions()
        };

        let result = ToolsListResult { tools };

        serde_json::to_value(result).map_err(|e| McpError::ProtocolError {
            reason: format!("failed to serialize tools list: {}", e),
        })
    }

    /// Handles the `tools/call` method with semaphore-based concurrency limiting.
    /// Dispatches to the appropriate store/memory method via dispatch_tool.
    async fn handle_tools_call(&self, request: &JsonRpcRequest) -> Result<Value, McpError> {
        // Try to acquire a semaphore permit for concurrency limiting
        let _permit = match self.semaphore.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                return Err(McpError::DispatchError {
                    reason: format!(
                        "Rate limited: maximum {} concurrent tool calls exceeded",
                        MAX_CONCURRENT_TOOL_CALLS
                    ),
                });
            }
        };

        // Extract tool name from params
        let tool_name = request
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("unknown");

        debug!("Tool call: {}", tool_name);

        let start = Instant::now();

        // Extract arguments from params
        let arguments = request
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Dispatch to the appropriate handler
        let result = super::dispatch::dispatch_tool(&self.store, tool_name, &arguments);

        info!(
            tool = %tool_name,
            duration_ms = %start.elapsed().as_millis(),
            "tool_call_complete"
        );

        result
    }

    /// Returns a reference to the store manager (for use by dispatch in Task 24).
    pub fn store(&self) -> &Arc<StoreManager> {
        &self.store
    }

    /// Returns a reference to the semaphore (for use by dispatch in Task 24).
    pub fn semaphore(&self) -> &Arc<Semaphore> {
        &self.semaphore
    }
}

// ---------------------------------------------------------------------------
// Tool Definitions
// ---------------------------------------------------------------------------

/// Returns the complete list of tool definitions exposed by the MCP server.
pub fn get_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        // Meta-tool: single-call code intelligence
        ToolDefinition {
            name: "ask".to_string(),
            description: "Single-call code intelligence. Ask a natural language question about the codebase and Cortex auto-routes to the right internal tools, composing a unified answer.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "Natural language question about the codebase (e.g. 'what calls validate_token', 'explain StoreManager', 'find dead code', 'show architecture')" }
                },
                "required": ["question"]
            }),
        },
        // Core Structural Tools
        ToolDefinition {
            name: "search_symbols".to_string(),
            description: "Search for symbols by name pattern with optional kind filter".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Symbol name pattern (glob supported)" },
                    "kind": { "type": "string", "description": "Optional node kind filter (Function, Class, Module, etc.)" },
                    "limit": { "type": "integer", "description": "Maximum results to return", "default": 50 }
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: "trace_callers".to_string(),
            description: "Trace all callers of a function/method up to a configurable depth".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "Fully qualified name of the target symbol" },
                    "depth": { "type": "integer", "description": "Maximum traversal depth (1-5)", "default": 3 }
                },
                "required": ["fqn"]
            }),
        },
        ToolDefinition {
            name: "trace_callees".to_string(),
            description: "Trace all callees of a function/method up to a configurable depth".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "Fully qualified name of the target symbol" },
                    "depth": { "type": "integer", "description": "Maximum traversal depth (1-5)", "default": 3 }
                },
                "required": ["fqn"]
            }),
        },
        ToolDefinition {
            name: "get_file_context".to_string(),
            description: "Get all symbols and relationships in a specific file".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Repository-relative file path" }
                },
                "required": ["file"]
            }),
        },
        ToolDefinition {
            name: "get_architecture".to_string(),
            description: "Get high-level architecture summary with module counts and entry points".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "find_dead_code".to_string(),
            description: "Find functions and methods with zero callers (excluding entry points)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Maximum results to return", "default": 50 }
                }
            }),
        },
        ToolDefinition {
            name: "detect_changes".to_string(),
            description: "Detect code changes since a git commit or timestamp with risk scores".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "since": { "type": "string", "description": "Git commit hash or Unix timestamp" }
                },
                "required": ["since"]
            }),
        },
        ToolDefinition {
            name: "blast_radius".to_string(),
            description: "Find all nodes that transitively depend on a given symbol".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "Fully qualified name of the target symbol" },
                    "depth": { "type": "integer", "description": "Maximum traversal depth (1-5)", "default": 3 }
                },
                "required": ["fqn"]
            }),
        },
        ToolDefinition {
            name: "get_code_snippet".to_string(),
            description: "Get the source code for a specific symbol by FQN".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "Fully qualified name of the symbol" }
                },
                "required": ["fqn"]
            }),
        },
        ToolDefinition {
            name: "query_graph".to_string(),
            description: "Execute a Cypher-like query over the structural graph".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Cypher-like query string (MATCH, WHERE, RETURN, LIMIT, ORDER BY)" }
                },
                "required": ["query"]
            }),
        },
        // HTTP Tools
        ToolDefinition {
            name: "get_http_routes".to_string(),
            description: "List HTTP route definitions filterable by service, method, and path".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "service": { "type": "string", "description": "Optional service name filter" },
                    "method": { "type": "string", "description": "Optional HTTP method filter (GET, POST, etc.)" },
                    "path_prefix": { "type": "string", "description": "Optional path prefix filter" }
                }
            }),
        },
        ToolDefinition {
            name: "trace_http_call".to_string(),
            description: "Trace an HTTP call to its route definition and all call sites".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url_pattern": { "type": "string", "description": "URL pattern to trace" }
                },
                "required": ["url_pattern"]
            }),
        },
        // Memory Tools
        ToolDefinition {
            name: "write_observation".to_string(),
            description: "Write an observation linked to a code symbol for cross-session memory".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "node_fqn": { "type": "string", "description": "FQN of the code symbol to attach the observation to" },
                    "observation_text": { "type": "string", "description": "The observation text to store" },
                    "agent_id": { "type": "string", "description": "Identifier of the agent writing the observation" }
                },
                "required": ["node_fqn", "observation_text"]
            }),
        },
        ToolDefinition {
            name: "read_observations".to_string(),
            description: "Read observations for a code symbol, with staleness status".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "FQN of the code symbol" },
                    "include_stale": { "type": "boolean", "description": "Whether to include stale observations", "default": false }
                },
                "required": ["fqn"]
            }),
        },
        ToolDefinition {
            name: "write_adr".to_string(),
            description: "Write an Architectural Decision Record".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "ADR title" },
                    "body": { "type": "string", "description": "ADR body text" },
                    "status": { "type": "string", "description": "Status: proposed, accepted, or deprecated", "default": "proposed" },
                    "linked_fqn": { "type": "string", "description": "Optional FQN to link the ADR to" }
                },
                "required": ["title", "body"]
            }),
        },
        ToolDefinition {
            name: "read_adrs".to_string(),
            description: "Read Architectural Decision Records with optional filters".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "Optional FQN filter" },
                    "status": { "type": "string", "description": "Optional status filter (proposed, accepted, deprecated)" }
                }
            }),
        },
        ToolDefinition {
            name: "prune_observations".to_string(),
            description: "Archive stale observations older than a threshold".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "older_than_days": { "type": "integer", "description": "Archive stale observations older than this many days", "default": 30 }
                }
            }),
        },
        // Security Tools
        ToolDefinition {
            name: "find_taint_paths".to_string(),
            description: "Find data flow paths from taint sources to sinks".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source_kind": { "type": "string", "description": "Taint source kind (HttpInput, FileInput, EnvVar, UserSession)" },
                    "sink_kind": { "type": "string", "description": "Taint sink kind (SqlQuery, CommandExecution, FileWrite, HttpResponse, LogOutput)" }
                }
            }),
        },
        ToolDefinition {
            name: "scan_owasp".to_string(),
            description: "Scan for OWASP Top 10 vulnerability patterns in the codebase".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "category": { "type": "string", "description": "Optional OWASP category filter (A01-A10)" }
                }
            }),
        },
        ToolDefinition {
            name: "generate_sbom".to_string(),
            description: "Generate a Software Bill of Materials from the import graph".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "format": { "type": "string", "description": "Output format (spdx)", "default": "spdx" }
                }
            }),
        },
        // Search Tools
        ToolDefinition {
            name: "search_text".to_string(),
            description: "Full-text search over symbol names and attributes using FTS5 with BM25 ranking".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text" },
                    "limit": { "type": "integer", "description": "Maximum results to return", "default": 20 }
                },
                "required": ["query"]
            }),
        },
        // Semantic Search (optional)
        ToolDefinition {
            name: "semantic_search".to_string(),
            description: "Semantic vector search for functionally similar code (requires semantic search to be enabled)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language or code query" },
                    "top_k": { "type": "integer", "description": "Number of results to return", "default": 10 }
                },
                "required": ["query"]
            }),
        },
        // Steering Tool
        ToolDefinition {
            name: "generate_steering".to_string(),
            description: "Generate CLAUDE.md, AGENTS.md, or .cursorrules content from graph analysis".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "format": { "type": "string", "description": "Output format: claude, agents, or cursorrules", "default": "claude" }
                }
            }),
        },
        // Graph Intelligence Tools
        ToolDefinition {
            name: "decompose_boundaries".to_string(),
            description: "Run Leiden community detection on the call graph to identify module boundaries and suggest decomposition".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "module_path": { "type": "string", "description": "Optional file path prefix to filter analysis scope (e.g., 'src/auth')" },
                    "coupling_threshold": { "type": "number", "description": "Minimum modularity gain threshold (0.0-1.0, default 0.5)", "default": 0.5 }
                }
            }),
        },
        ToolDefinition {
            name: "get_complexity_hotspots".to_string(),
            description: "Find functions with highest cyclomatic complexity, sorted by complexity descending".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Maximum results to return", "default": 20 },
                    "threshold": { "type": "integer", "description": "Minimum complexity threshold to include", "default": 5 }
                }
            }),
        },
        // Task-Aware Context Budgeting
        ToolDefinition {
            name: "get_task_context".to_string(),
            description: "Returns the most relevant structural context for a task within a token budget".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["task_description", "token_budget"],
                "properties": {
                    "task_description": {
                        "type": "string",
                        "description": "Natural language description of the task"
                    },
                    "token_budget": {
                        "type": "integer",
                        "description": "Maximum tokens to include in the response",
                        "minimum": 100,
                        "maximum": 100000
                    },
                    "include_code": {
                        "type": "boolean",
                        "description": "Include source code snippets for top symbols",
                        "default": false
                    },
                    "scope": {
                        "type": "string",
                        "description": "File path or directory prefix to constrain search"
                    }
                }
            }),
        },
        // Dependency Vulnerability Check
        ToolDefinition {
            name: "check_dependencies".to_string(),
            description: "Check dependencies against OSV.dev for known vulnerabilities".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string", "description": "Optional repository root path" }
                }
            }),
        },
        // Class Hierarchy
        ToolDefinition {
            name: "get_class_hierarchy".to_string(),
            description: "Query class inheritance and interface implementation tree".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "Fully qualified name of the class or interface" },
                    "direction": { "type": "string", "description": "Direction: 'both' (default), 'up' (parents only), or 'down' (children only)", "default": "both" }
                },
                "required": ["fqn"]
            }),
        },
        // Git Hotspots
        ToolDefinition {
            name: "get_git_hotspots".to_string(),
            description: "Find high-churn files ranked by risk score (git commit frequency combined with caller count)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Maximum results to return", "default": 20 },
                    "since_months": { "type": "integer", "description": "How far back in git history to look", "default": 6 }
                }
            }),
        },
        // Import Graph
        ToolDefinition {
            name: "get_import_graph".to_string(),
            description: "Get all import relationships for a file or module".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Specific file path to get imports for" },
                    "module": { "type": "string", "description": "Module name prefix to filter imports" }
                }
            }),
        },
        // Similar Functions
        ToolDefinition {
            name: "find_similar_functions".to_string(),
            description: "Find functions with similar call patterns (overlapping callee sets)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "Fully qualified name of the target function" },
                    "limit": { "type": "integer", "description": "Maximum results to return", "default": 5 }
                },
                "required": ["fqn"]
            }),
        },
    ]
}

/// Returns the reduced set of 5 core tools for smart-tools mode.
/// This reduces context window overhead by ~89% while still providing
/// full functionality through the `ask` meta-tool.
pub fn get_smart_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "ask".to_string(),
            description: "Single-call code intelligence. Ask a natural language question about the codebase and Cortex auto-routes to the right internal tools, composing a unified answer. Handles: callers, callees, blast radius, explain, search, security, dead code, architecture.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "Natural language question about the codebase (e.g. 'what calls validate_token', 'explain StoreManager', 'find dead code', 'show architecture')" }
                },
                "required": ["question"]
            }),
        },
        ToolDefinition {
            name: "search_symbols".to_string(),
            description: "Search for symbols by name pattern with optional kind filter".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Symbol name pattern (glob supported)" },
                    "kind": { "type": "string", "description": "Optional node kind filter (Function, Class, Module, etc.)" },
                    "limit": { "type": "integer", "description": "Maximum results to return", "default": 50 }
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: "write_observation".to_string(),
            description: "Write an observation linked to a code symbol for cross-session memory".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "node_fqn": { "type": "string", "description": "FQN of the code symbol to attach the observation to" },
                    "observation_text": { "type": "string", "description": "The observation text to store" },
                    "agent_id": { "type": "string", "description": "Identifier of the agent writing the observation" }
                },
                "required": ["node_fqn", "observation_text"]
            }),
        },
        ToolDefinition {
            name: "read_observations".to_string(),
            description: "Read observations for a code symbol, with staleness status".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fqn": { "type": "string", "description": "FQN of the code symbol" },
                    "include_stale": { "type": "boolean", "description": "Whether to include stale observations", "default": false }
                },
                "required": ["fqn"]
            }),
        },
        ToolDefinition {
            name: "get_architecture".to_string(),
            description: "Get high-level architecture summary with module counts and entry points".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a test request JSON string.
    fn make_request(id: Option<i64>, method: &str, params: Option<Value>) -> String {
        let mut obj = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if let Some(id_val) = id {
            obj["id"] = serde_json::json!(id_val);
        }
        if let Some(p) = params {
            obj["params"] = p;
        }
        serde_json::to_string(&obj).unwrap()
    }

    /// Helper to create a McpServer with a temporary store for testing.
    fn create_test_server() -> (McpServer, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let store = Arc::new(StoreManager::new(tmp.path()).expect("failed to create store"));
        // Apply migrations so dispatch can query tables
        {
            let conn = store.write_conn();
            crate::store::migrations::run_migrations(
                &conn,
                std::path::Path::new("migrations"),
            )
            .expect("failed to run migrations");
        }
        let server = McpServer::new(store);
        (server, tmp)
    }

    #[tokio::test]
    async fn test_initialize_response() {
        let (server, _tmp) = create_test_server();
        let line = make_request(Some(1), "initialize", Some(serde_json::json!({})));

        let response = server.handle_line(&line).await.unwrap();

        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, Some(serde_json::json!(1)));
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(result["serverInfo"]["version"], SERVER_VERSION);
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
    }

    #[tokio::test]
    async fn test_tools_list_returns_all_tools() {
        let (server, _tmp) = create_test_server();
        let line = make_request(Some(2), "tools/list", None);

        let response = server.handle_line(&line).await.unwrap();

        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();

        // Verify all expected tools are present
        let tool_names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();

        let expected_tools = [
            "search_symbols",
            "trace_callers",
            "trace_callees",
            "get_file_context",
            "get_architecture",
            "find_dead_code",
            "detect_changes",
            "blast_radius",
            "get_code_snippet",
            "query_graph",
            "get_http_routes",
            "trace_http_call",
            "write_observation",
            "read_observations",
            "write_adr",
            "read_adrs",
            "prune_observations",
            "find_taint_paths",
            "scan_owasp",
            "generate_sbom",
            "search_text",
            "semantic_search",
            "generate_steering",
            "decompose_boundaries",
            "get_complexity_hotspots",
            "get_task_context",
        ];

        for expected in &expected_tools {
            assert!(
                tool_names.contains(expected),
                "Missing tool: {}",
                expected
            );
        }

        // Verify each tool has required fields
        for tool in tools {
            assert!(tool["name"].is_string(), "Tool missing name");
            assert!(tool["description"].is_string(), "Tool missing description");
            assert!(tool["inputSchema"].is_object(), "Tool missing inputSchema");
        }
    }

    #[tokio::test]
    async fn test_malformed_json_returns_parse_error() {
        let (server, _tmp) = create_test_server();
        let line = "this is not valid json{{{";

        let response = server.handle_line(line).await.unwrap();

        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.id.is_none());
        assert!(response.result.is_none());

        let error = response.error.unwrap();
        assert_eq!(error.code, PARSE_ERROR);
        assert!(error.message.starts_with("Parse error:"));
    }

    #[tokio::test]
    async fn test_unknown_method_returns_method_not_found() {
        let (server, _tmp) = create_test_server();
        let line = make_request(Some(99), "nonexistent/method", None);

        let response = server.handle_line(&line).await.unwrap();

        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, Some(serde_json::json!(99)));
        assert!(response.result.is_none());

        let error = response.error.unwrap();
        assert_eq!(error.code, METHOD_NOT_FOUND);
        assert!(error.message.contains("nonexistent/method"));
    }

    #[tokio::test]
    async fn test_notification_returns_no_response() {
        let (server, _tmp) = create_test_server();
        // Notification has no id
        let line = make_request(None, "notifications/initialized", None);

        let response = server.handle_line(&line).await;
        assert!(response.is_none(), "Notifications should not produce a response");
    }

    #[tokio::test]
    async fn test_tools_call_stub() {
        let (server, _tmp) = create_test_server();
        let line = make_request(
            Some(3),
            "tools/call",
            Some(serde_json::json!({
                "name": "search_symbols",
                "arguments": { "pattern": "test" }
            })),
        );

        let response = server.handle_line(&line).await.unwrap();

        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, Some(serde_json::json!(3)));
        // Stub returns a result (not an error at the JSON-RPC level)
        assert!(response.result.is_some());
    }

    #[tokio::test]
    async fn test_empty_line_ignored() {
        let (server, _tmp) = create_test_server();
        let response = server.handle_line("").await;
        // Empty lines should be handled gracefully - in the run loop they're skipped
        // but handle_line will try to parse and return parse error
        // Actually, the run() method skips empty lines before calling handle_line
        // So handle_line receiving empty string will return parse error
        assert!(response.is_some());
    }

    #[tokio::test]
    async fn test_server_does_not_crash_on_malformed_input() {
        let (server, _tmp) = create_test_server();

        // Various malformed inputs
        let inputs = vec![
            "null",
            "42",
            "[]",
            "\"just a string\"",
            "{\"jsonrpc\": \"2.0\"}",
            "{\"incomplete",
            "\x00\x01\x02",
        ];

        for input in inputs {
            let response = server.handle_line(input).await;
            // Should either return a parse error or a valid response, never panic
            if let Some(resp) = response {
                assert_eq!(resp.jsonrpc, "2.0");
            }
        }
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let (server, _tmp) = create_test_server();

        // Acquire all semaphore permits manually
        let permits: Vec<_> = (0..MAX_CONCURRENT_TOOL_CALLS)
            .map(|_| server.semaphore.try_acquire().unwrap())
            .collect();

        // Now a tools/call should fail with rate limit
        let line = make_request(
            Some(10),
            "tools/call",
            Some(serde_json::json!({
                "name": "search_symbols",
                "arguments": { "pattern": "test" }
            })),
        );

        let response = server.handle_line(&line).await.unwrap();
        assert!(response.result.is_none());
        let error = response.error.unwrap();
        assert_eq!(error.code, INTERNAL_ERROR);
        assert!(error.message.contains("Rate limited"));

        // Drop permits to release
        drop(permits);
    }
}
