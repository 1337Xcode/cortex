//! Cortex's MCP server, which an agent starts as `cortex mcp` and talks to over stdio. It offers
//! `cortex_where`: the code the retrieval engine finds for a question, with its reasons and
//! confidence, as `cortex where` prints it; and `cortex_note`: a project rule the agent found, kept
//! as a candidate fact. Every call is answered by the repository's daemon through [`Daemon`],
//! which the binary implements; this crate knows the protocol, the tools and their limits, not how
//! the daemon is found or started.

mod cap;
mod server;

pub use server::{Daemon, Server, serve};

/// What can stop the server.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// The runtime the server runs on could not be built.
    #[error("starting the server's runtime")]
    Runtime(#[source] std::io::Error),
    /// The client's first messages were not an MCP session's start.
    #[error("starting the MCP session")]
    Start(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The session ended with an error.
    #[error("running the MCP session")]
    Session(#[source] Box<dyn std::error::Error + Send + Sync>),
}
