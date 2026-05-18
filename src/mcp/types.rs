//! JSON-RPC 2.0 types and MCP protocol types for the Cortex MCP server.
//!
//! Defines the wire format for JSON-RPC requests and responses as well as
//! standard error codes per the JSON-RPC 2.0 specification.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Standard JSON-RPC 2.0 error codes
// ---------------------------------------------------------------------------

/// Parse error: Invalid JSON was received by the server.
pub const PARSE_ERROR: i32 = -32700;

/// Invalid Request: The JSON sent is not a valid Request object.
pub const INVALID_REQUEST: i32 = -32600;

/// Method not found: The method does not exist or is not available.
pub const METHOD_NOT_FOUND: i32 = -32601;

/// Invalid params: Invalid method parameter(s).
pub const INVALID_PARAMS: i32 = -32602;

/// Internal error: Internal JSON-RPC error.
pub const INTERNAL_ERROR: i32 = -32603;

/// Rate limited: Too many concurrent tool calls.
pub const RATE_LIMITED: i32 = -32003;

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 Request
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 request object.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    /// Protocol version, must be "2.0".
    pub jsonrpc: String,

    /// Request identifier. Notifications have no id.
    pub id: Option<serde_json::Value>,

    /// Method name to invoke.
    pub method: String,

    /// Optional parameters for the method.
    pub params: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 Response
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 response object.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    /// Protocol version, always "2.0".
    pub jsonrpc: String,

    /// Request identifier matching the request.
    pub id: Option<serde_json::Value>,

    /// Successful result (mutually exclusive with error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,

    /// Error object (mutually exclusive with result).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 Error
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    /// Numeric error code.
    pub code: i32,

    /// Human-readable error message.
    pub message: String,

    /// Optional additional data about the error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Helper constructors
// ---------------------------------------------------------------------------

impl JsonRpcResponse {
    /// Creates a successful response with the given result.
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Creates an error response with the given error details.
    pub fn error(id: Option<serde_json::Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }

    /// Creates an error response with additional data.
    pub fn error_with_data(
        id: Option<serde_json::Value>,
        code: i32,
        message: String,
        data: serde_json::Value,
    ) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: Some(data),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// MCP Protocol Types
// ---------------------------------------------------------------------------

/// MCP server capabilities returned during initialization.
#[derive(Debug, Serialize)]
pub struct ServerCapabilities {
    /// Tools capability indicating the server supports tool listing and calling.
    pub tools: ToolsCapability,
}

/// Indicates the server supports tools.
#[derive(Debug, Serialize)]
pub struct ToolsCapability {
    /// Whether the tool list may change over time.
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

/// MCP server info returned during initialization.
#[derive(Debug, Serialize)]
pub struct ServerInfo {
    /// Server name.
    pub name: String,

    /// Server version.
    pub version: String,
}

/// MCP initialize response result.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// Protocol version supported by the server.
    pub protocol_version: String,

    /// Server capabilities.
    pub capabilities: ServerCapabilities,

    /// Server identification.
    pub server_info: ServerInfo,
}

/// A tool definition exposed via MCP tools/list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    /// Tool name (e.g., "search_symbols").
    pub name: String,

    /// Human-readable description of what the tool does.
    pub description: String,

    /// JSON Schema describing the tool's input parameters.
    pub input_schema: serde_json::Value,
}

/// Response for tools/list method.
#[derive(Debug, Serialize)]
pub struct ToolsListResult {
    /// List of available tools.
    pub tools: Vec<ToolDefinition>,
}

/// Response for tools/call method (stub for now).
#[derive(Debug, Serialize)]
pub struct ToolCallResult {
    /// Tool execution content.
    pub content: Vec<ToolContent>,

    /// Whether the tool call resulted in an error.
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

/// Content block in a tool call response.
#[derive(Debug, Serialize)]
pub struct ToolContent {
    /// Content type (always "text" for now).
    #[serde(rename = "type")]
    pub content_type: String,

    /// Text content.
    pub text: String,
}
