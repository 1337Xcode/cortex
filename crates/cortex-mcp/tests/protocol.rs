//! The server spoken to by an MCP client over an in-memory pipe, its daemon a fake that records
//! each request and answers as told: the one tool and its schema, the answer as the daemon wrote
//! it, the inputs refused, and each way the daemon can fail turned into a tool error the agent
//! reads.

use std::sync::{Arc, Mutex};

use cortex_core::ipc::{RequestBody, ResponseBody};
use cortex_mcp::{Daemon, Server};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

/// What a test's result is.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A daemon that answers every request with one answer and keeps the requests it was sent.
struct Fake {
    /// The answer, or why there is none.
    answer: Result<ResponseBody, String>,
    /// Every request, in order.
    asked: Mutex<Vec<RequestBody>>,
}

impl Daemon for Fake {
    fn ask(&self, body: RequestBody) -> Result<ResponseBody, String> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(body);
        }
        self.answer.clone()
    }
}

/// Runs `calls` against a server whose daemon answers `answer`, and returns, for each call, the
/// result as JSON (or the protocol error's code), with the requests the daemon was sent.
async fn session(
    answer: Result<ResponseBody, String>,
    calls: Vec<Value>,
) -> TestResult<(Vec<Value>, Vec<RequestBody>)> {
    let fake = Arc::new(Fake {
        answer,
        asked: Mutex::new(Vec::new()),
    });
    let (server_side, client_side) = tokio::io::duplex(64 * 1024);
    let server = Server::new(fake.clone()).serve(server_side);
    let client = ().serve(client_side);
    let (server, client) = tokio::join!(server, client);
    let (server, client) = (server?, client?);
    let mut results = Vec::new();
    for arguments in calls {
        let Value::Object(arguments) = arguments else {
            return Err("arguments are no object".into());
        };
        let params = CallToolRequestParams::new("cortex_where").with_arguments(arguments);
        results.push(match client.call_tool(params).await {
            Ok(result) => serde_json::to_value(result)?,
            Err(rmcp::ServiceError::McpError(error)) => json!({"code": error.code.0}),
            Err(other) => return Err(other.into()),
        });
    }
    client.cancel().await?;
    server.cancel().await?;
    let asked = fake.asked.lock().map_err(|_| "poisoned")?.clone();
    Ok((results, asked))
}

/// A tool result holding `text`, a success or an error as `is_error` says.
fn result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

/// The server offers exactly one tool, `cortex_where`, taking a query and an optional limit.
#[tokio::test]
async fn the_one_tool_is_cortex_where() -> TestResult {
    let fake = Arc::new(Fake {
        answer: Err("unused".to_owned()),
        asked: Mutex::new(Vec::new()),
    });
    let (server_side, client_side) = tokio::io::duplex(64 * 1024);
    let (server, client) =
        tokio::join!(Server::new(fake).serve(server_side), ().serve(client_side));
    let (server, client) = (server?, client?);
    let tools = client.list_all_tools().await?;
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, ["cortex_where"]);
    let schema = serde_json::to_value(&tools.first().ok_or("no tool")?.input_schema)?;
    assert_eq!(schema.pointer("/required"), Some(&json!(["query"])));
    assert_eq!(
        schema.pointer("/properties/query/type"),
        Some(&json!("string"))
    );
    assert_eq!(schema.pointer("/properties/limit/minimum"), Some(&json!(1)));
    assert_eq!(schema.pointer("/properties/limit/maximum"), Some(&json!(5)));
    let info = serde_json::to_value(client.peer_info())?;
    assert_eq!(info.pointer("/serverInfo/name"), Some(&json!("cortex")));
    client.cancel().await?;
    server.cancel().await?;
    Ok(())
}

/// A question goes to the daemon as `cortex where` sends it, the limit 3 when left out, and its
/// answer comes back as the daemon wrote it; a limit outside 1 to 5 and a blank question are tool
/// errors the model can correct, and never reach the daemon.
#[tokio::test]
async fn a_question_is_answered_as_the_daemon_wrote_it() -> TestResult {
    let answer = "calc/ops.py:12-13 divide (callers 2, confidence 0.8)\n    def divide(a, b):\n";
    let (results, asked) = session(
        Ok(ResponseBody::Reply {
            text: Some(answer.to_owned()),
        }),
        vec![
            json!({"query": "where is divide"}),
            json!({"query": "divide", "limit": 5}),
            json!({"query": "divide", "limit": 6}),
            json!({"query": "divide", "limit": 0}),
            json!({"query": "divide", "limit": 300}),
            json!({"query": "divide", "limit": -1}),
            json!({"query": "  "}),
        ],
    )
    .await?;
    let bad_limit = result("Cortex: the limit is a number from 1 to 5", true);
    assert_eq!(
        results,
        [
            result(answer, false),
            result(answer, false),
            bad_limit.clone(),
            bad_limit.clone(),
            bad_limit.clone(),
            bad_limit,
            result("Cortex: the query is empty: ask in a sentence", true),
        ]
    );
    assert_eq!(
        asked,
        [
            RequestBody::Where {
                query: "where is divide".to_owned(),
                limit: 3
            },
            RequestBody::Where {
                query: "divide".to_owned(),
                limit: 5
            },
        ]
    );
    Ok(())
}

/// Each answer without code is said in words: nothing found, the daemon's error, no daemon; the
/// last two as tool errors, which the agent reads, while the session goes on.
#[tokio::test]
async fn an_answer_without_code_is_said_in_words() -> TestResult {
    let question = || vec![json!({"query": "divide"})];
    let (nothing, _) = session(Ok(ResponseBody::Reply { text: None }), question()).await?;
    assert_eq!(
        nothing,
        [result(
            "Nothing found: no file the engine can show for this question, or the daemon has \
             not built its graph yet (`cortex status` says which).",
            false
        )]
    );
    let (error, _) = session(
        Ok(ResponseBody::Error {
            message: "the index is not ready".to_owned(),
        }),
        question(),
    )
    .await?;
    assert_eq!(
        error,
        [result(
            "Cortex: the daemon could not answer: the index is not ready",
            true
        )]
    );
    let (down, _) = session(
        Err("no daemon answered within 20 s".to_owned()),
        vec![json!({"query": "divide"}), json!({"query": "add"})],
    )
    .await?;
    let failed = result("Cortex: no daemon answered within 20 s", true);
    assert_eq!(down, [failed.clone(), failed]);
    Ok(())
}

/// An answer past 1,200 tokens is cut after a whole line, with the note saying so.
#[tokio::test]
async fn a_long_answer_is_cut() -> TestResult {
    let line = "x".repeat(99);
    let long: String = (0..300).map(|_| format!("{line}\n")).collect();
    let (results, _) = session(
        Ok(ResponseBody::Reply { text: Some(long) }),
        vec![json!({"query": "divide"})],
    )
    .await?;
    let text = results
        .first()
        .and_then(|result| result.pointer("/content/0/text"))
        .and_then(Value::as_str)
        .ok_or("no text")?;
    assert_eq!(text.lines().count(), 48);
    assert!(
        text.ends_with("[cortex] cut at 1200 tokens: 253 more lines; ask with a smaller limit\n")
    );
    Ok(())
}
