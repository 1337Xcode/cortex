//! The server spoken to by an MCP client over an in-memory pipe, its daemon a fake that records
//! each request and answers as told: the tools and their schemas, the answer as the daemon wrote
//! it, the inputs refused, and each way the daemon can fail turned into a tool error the agent
//! reads.

use std::sync::{Arc, Mutex};

use cortex_core::ipc::{FactsAnswer, Note, NoteOutcome, RequestBody, ResponseBody};
use cortex_core::{FactKind, FactSource};
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

/// Runs `calls` of `cortex_where` against a server whose daemon answers `answer`, and returns,
/// for each call, the result as JSON (or the protocol error's code), with the requests the daemon
/// was sent.
async fn session(
    answer: Result<ResponseBody, String>,
    calls: Vec<Value>,
) -> TestResult<(Vec<Value>, Vec<RequestBody>)> {
    session_of("cortex_where", answer, calls).await
}

/// Runs `calls` of the tool `tool` against a server whose daemon answers `answer`, as [`session`]
/// does.
async fn session_of(
    tool: &'static str,
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
        let params = CallToolRequestParams::new(tool).with_arguments(arguments);
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

/// The input schema of the tool `name` among `tools`.
fn schema_of_tool(tools: &[rmcp::model::Tool], name: &str) -> TestResult<Value> {
    let tool = tools
        .iter()
        .find(|tool| tool.name == name)
        .ok_or("no such tool")?;
    Ok(serde_json::to_value(&tool.input_schema)?)
}

/// The server offers exactly three tools: `cortex_note`, taking a rule, its anchors and an
/// optional kind; `cortex_where`, taking a query and an optional limit; and `cortex_why`, taking a
/// target.
#[tokio::test]
async fn the_tools_are_cortex_note_where_and_why() -> TestResult {
    let fake = Arc::new(Fake {
        answer: Err("unused".to_owned()),
        asked: Mutex::new(Vec::new()),
    });
    let (server_side, client_side) = tokio::io::duplex(64 * 1024);
    let (server, client) =
        tokio::join!(Server::new(fake).serve(server_side), ().serve(client_side));
    let (server, client) = (server?, client?);
    let tools = client.list_all_tools().await?;
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(names, ["cortex_note", "cortex_where", "cortex_why"]);
    let why = schema_of_tool(&tools, "cortex_why")?;
    assert_eq!(why.pointer("/required"), Some(&json!(["target"])));
    assert_eq!(
        why.pointer("/properties/target/type"),
        Some(&json!("string"))
    );
    let note = schema_of_tool(&tools, "cortex_note")?;
    assert_eq!(note.pointer("/required"), Some(&json!(["text", "anchors"])));
    assert_eq!(
        note.pointer("/properties/anchors/type"),
        Some(&json!("array"))
    );
    let schema = schema_of_tool(&tools, "cortex_where")?;
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
            to_user: None,
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
                limit: 3,
                worktree_root: None
            },
            RequestBody::Where {
                query: "divide".to_owned(),
                limit: 5,
                worktree_root: None
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
    let (nothing, _) = session(
        Ok(ResponseBody::Reply {
            text: None,
            to_user: None,
        }),
        question(),
    )
    .await?;
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
        Ok(ResponseBody::Reply {
            text: Some(long),
            to_user: None,
        }),
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

/// A rule goes to the daemon as an agent's note, a convention when no kind is given, its anchors
/// as given; what became of it comes back in words: a new candidate, a fact that says it already,
/// one that is now active. A kind Cortex does not know, an empty rule and no anchor are tool
/// errors the model can correct, and never reach the daemon.
#[tokio::test]
async fn a_rule_is_noted_as_an_agent_s_candidate() -> TestResult {
    let calls = || {
        vec![
            json!({"text": "Money is kept in cents.", "anchors": ["calc/money.py:Money"]}),
            json!({"text": "Tax rounds up.", "anchors": ["calc/tax.py"], "kind": "pitfall"}),
            json!({"text": "Tax rounds up.", "anchors": ["calc/tax.py"], "kind": "rule"}),
            json!({"text": " ", "anchors": ["calc/tax.py"]}),
            json!({"text": "Tax rounds up.", "anchors": []}),
        ]
    };
    let noted = |outcome| Ok(ResponseBody::Facts(FactsAnswer::Noted { id: 7, outcome }));
    let (results, asked) = session_of("cortex_note", noted(NoteOutcome::Added), calls()).await?;
    let added = result(
        "Noted as fact 7, a candidate: given to agents once the user confirms it or it is noted \
         again in another task.",
        false,
    );
    assert_eq!(
        results,
        [
            added.clone(),
            added,
            result(
                "Cortex: the kind is convention, decision, invariant, pitfall or ownership",
                true
            ),
            result("Cortex: the rule is empty: say it in a sentence", true),
            result(
                "Cortex: name what the rule is about: a file, `path:symbol`, or `.` for the \
                 repository",
                true
            ),
        ]
    );
    assert_eq!(
        asked,
        [
            RequestBody::Note(Note {
                text: "Money is kept in cents.".to_owned(),
                kind: FactKind::Convention,
                source: FactSource::Agent,
                at: vec!["calc/money.py:Money".to_owned()],
            }),
            RequestBody::Note(Note {
                text: "Tax rounds up.".to_owned(),
                kind: FactKind::Pitfall,
                source: FactSource::Agent,
                at: vec!["calc/tax.py".to_owned()],
            }),
        ]
    );
    let one = || vec![json!({"text": "Tax rounds up.", "anchors": ["calc/tax.py"]})];
    let (merged, _) = session_of("cortex_note", noted(NoteOutcome::Merged), one()).await?;
    assert_eq!(merged, [result("Fact 7 says this already.", false)]);
    let (activated, _) = session_of("cortex_note", noted(NoteOutcome::Activated), one()).await?;
    assert_eq!(
        activated,
        [result(
            "Fact 7 says this already, and is now active.",
            false
        )]
    );
    let refused = Ok(ResponseBody::Error {
        message: "calc/tax.py is not a file Cortex indexes".to_owned(),
    });
    let (failed, _) = session_of("cortex_note", refused, one()).await?;
    assert_eq!(
        failed,
        [result(
            "Cortex: the rule was not noted: calc/tax.py is not a file Cortex indexes",
            true
        )]
    );
    Ok(())
}

/// A target goes to the daemon as it is given; the daemon's answer comes back as it wrote it, and
/// its refusal as a tool error the model can correct, in the daemon's own words. An empty target
/// is refused before the daemon is asked.
#[tokio::test]
async fn a_why_is_answered_as_the_daemon_wrote_it() -> TestResult {
    let answer = "[cortex] Why · calc/ops.py\nLinked by code:\n  - calc/tax.py (0.90)\n";
    let reply = Ok(ResponseBody::Reply {
        text: Some(answer.to_owned()),
        to_user: None,
    });
    let calls = vec![json!({"target": "calc/ops.py"}), json!({"target": " "})];
    let (results, asked) = session_of("cortex_why", reply, calls).await?;
    assert_eq!(
        results,
        [
            result(answer, false),
            result(
                "Cortex: name a file, `path:line`, `path:symbol` or a symbol's name",
                true
            ),
        ]
    );
    assert_eq!(
        asked,
        [RequestBody::Why {
            target: "calc/ops.py".to_owned(),
            worktree_root: None
        }]
    );
    let refused = Ok(ResponseBody::Error {
        message: "refund names no file or symbol Cortex indexes".to_owned(),
    });
    let one = vec![json!({"target": "refund"})];
    let (failed, _) = session_of("cortex_why", refused, one).await?;
    assert_eq!(
        failed,
        [result(
            "Cortex: refund names no file or symbol Cortex indexes",
            true
        )]
    );
    Ok(())
}
