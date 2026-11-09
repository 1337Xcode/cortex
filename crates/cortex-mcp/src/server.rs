//! The MCP server: its tools, `cortex_where` and `cortex_note`, answered by the daemon, and the
//! session over stdio. A call the server cannot answer is a tool error the agent reads, never a
//! crash: the session goes on.

use std::sync::Arc;

use cortex_core::ipc::{FactsAnswer, Note, NoteOutcome, RequestBody, ResponseBody};
use cortex_core::{FactKind, FactSource};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::McpError;
use crate::cap::capped;

/// How many files `cortex_where` names when the call does not say, as `cortex where` does.
const DEFAULT_LIMIT: u8 = 3;

/// The most files `cortex_where` names.
const MAX_LIMIT: u8 = 5;

/// What `cortex_where` says when the engine has nothing to show.
const NOTHING_FOUND: &str = "Nothing found: no file the engine can show for this question, or \
                             the daemon has not built its graph yet (`cortex status` says which).";

/// The repository's daemon, as the server asks it: the binary finds it, starts one when none
/// runs, and waits for its answer.
pub trait Daemon: Send + Sync + 'static {
    /// The daemon's answer to `body`, or a sentence for the agent saying why there is none (no
    /// repository here, no daemon could be started, it did not answer in time).
    ///
    /// # Errors
    ///
    /// Why the daemon gave no answer.
    fn ask(&self, body: RequestBody) -> Result<ResponseBody, String>;
}

/// The input of `cortex_where`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WhereInput {
    /// The question, in the words a prompt would use.
    query: String,
    /// The most files to name, from 1 to 5; 3 when left out.
    #[schemars(range(min = 1, max = 5))]
    limit: Option<i64>,
}

/// The input of `cortex_note`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NoteInput {
    /// The rule, one short sentence of at most 240 characters.
    text: String,
    /// What it is about: file paths, `path:symbol`, or `.` for the whole repository.
    anchors: Vec<String>,
    /// convention, decision, invariant, pitfall or ownership; convention when left out.
    kind: Option<String>,
}

/// The server: its tools, each answered by the daemon.
#[derive(Clone)]
pub struct Server {
    /// The daemon every call is forwarded to.
    daemon: Arc<dyn Daemon>,
}

#[tool_router]
impl Server {
    /// A server forwarding every call to `daemon`.
    #[must_use]
    pub fn new(daemon: Arc<dyn Daemon>) -> Self {
        Self { daemon }
    }

    /// `cortex_where`: the code the engine finds for a question, as `cortex where` prints it,
    /// within 1,200 tokens.
    #[tool(
        name = "cortex_where",
        description = "Find the code a question is about in this repository: exact file spans with reasons and confidence."
    )]
    async fn find_where(
        &self,
        Parameters(input): Parameters<WhereInput>,
    ) -> Result<CallToolResult, ErrorData> {
        // Input the model can correct is a tool error it reads, not a protocol error.
        let limit = match input.limit.map(u8::try_from) {
            None => DEFAULT_LIMIT,
            Some(Ok(limit)) if (1..=MAX_LIMIT).contains(&limit) => limit,
            Some(_) => return Ok(failed("the limit is a number from 1 to 5")),
        };
        if input.query.trim().is_empty() {
            return Ok(failed("the query is empty: ask in a sentence"));
        }
        let daemon = Arc::clone(&self.daemon);
        let body = RequestBody::Where {
            query: input.query,
            limit,
        };
        // The daemon is asked over a blocking connection, off the session's thread.
        let asked = tokio::task::spawn_blocking(move || daemon.ask(body)).await;
        let text = match asked {
            Ok(Ok(ResponseBody::Reply {
                text: Some(text), ..
            })) => capped(&text),
            Ok(Ok(ResponseBody::Reply { text: None, .. })) => NOTHING_FOUND.to_owned(),
            Ok(Ok(ResponseBody::Error { message })) => {
                return Ok(failed(&format!("the daemon could not answer: {message}")));
            }
            Ok(Ok(_)) => return Ok(failed("the daemon gave an answer of another kind")),
            Ok(Err(why)) => return Ok(failed(&why)),
            Err(_) => return Ok(failed("the call to the daemon stopped")),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    /// `cortex_note`: a project rule the agent found, kept as a candidate fact until the user
    /// confirms it or it is noted again in another task.
    #[tool(
        name = "cortex_note",
        description = "Record a non-obvious project rule about files or symbols, as a candidate fact."
    )]
    async fn note(
        &self,
        Parameters(input): Parameters<NoteInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let kind = match input.kind.as_deref() {
            None => FactKind::Convention,
            Some(name) => match FactKind::from_name(name) {
                Some(kind) => kind,
                None => {
                    return Ok(failed(
                        "the kind is convention, decision, invariant, pitfall or ownership",
                    ));
                }
            },
        };
        if input.text.trim().is_empty() {
            return Ok(failed("the rule is empty: say it in a sentence"));
        }
        if input.anchors.is_empty() {
            return Ok(failed(
                "name what the rule is about: a file, `path:symbol`, or `.` for the repository",
            ));
        }
        let daemon = Arc::clone(&self.daemon);
        let body = RequestBody::Note(Note {
            text: input.text,
            kind,
            source: FactSource::Agent,
            at: input.anchors,
        });
        let asked = tokio::task::spawn_blocking(move || daemon.ask(body)).await;
        let text = match asked {
            Ok(Ok(ResponseBody::Facts(FactsAnswer::Noted { id, outcome }))) => match outcome {
                NoteOutcome::Added => format!(
                    "Noted as fact {id}, a candidate: given to agents once the user confirms it \
                     or it is noted again in another task."
                ),
                NoteOutcome::Merged => format!("Fact {id} says this already."),
                NoteOutcome::Activated => {
                    format!("Fact {id} says this already, and is now active.")
                }
            },
            Ok(Ok(ResponseBody::Error { message })) => {
                return Ok(failed(&format!("the rule was not noted: {message}")));
            }
            Ok(Ok(_)) => return Ok(failed("the daemon gave an answer of another kind")),
            Ok(Err(why)) => return Ok(failed(&why)),
            Err(_) => return Ok(failed("the call to the daemon stopped")),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(capped(
            &text,
        ))]))
    }
}

#[tool_handler]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("cortex", env!("CARGO_PKG_VERSION")))
    }
}

/// A tool error the agent reads, saying `why` the call has no answer, within the cap.
fn failed(why: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(capped(&format!("Cortex: {why}")))])
}

/// Runs the server on stdio until the client ends the session, forwarding every call to
/// `daemon`.
///
/// # Errors
///
/// The runtime could not start, the client's first messages were not an MCP session's start, or
/// the session ended with an error.
pub fn serve(daemon: Arc<dyn Daemon>) -> Result<(), McpError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(McpError::Runtime)?;
    runtime.block_on(async move {
        let running = Server::new(daemon)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| McpError::Start(Box::new(error)))?;
        running
            .waiting()
            .await
            .map_err(|error| McpError::Session(Box::new(error)))?;
        Ok(())
    })
}
