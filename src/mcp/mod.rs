//! MCP server module.
//!
//! Implements the Model Context Protocol (MCP) over JSON-RPC 2.0 stdio transport.
//! The server reads newline-delimited JSON from stdin and writes responses to stdout.

pub mod ask;
pub mod dispatch;
pub mod health;
pub mod repo_brief;
pub mod savings_store;
pub mod server;
pub mod token_counter;
pub mod tool_surface;
pub mod types;
